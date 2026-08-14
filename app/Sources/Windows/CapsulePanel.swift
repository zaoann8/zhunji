// 胶囊 NSPanel — 移植自原版 show_capsule_window_no_activate（capsule_focus.rs 头部注释）：
// - level 25：菜单栏(24)之上，叠在全屏 app 之上
// - collectionBehavior 273 = CAN_JOIN_ALL_SPACES(1<<0) | STATIONARY(1<<4) |
//   FULL_SCREEN_AUXILIARY(1<<8)；入场帧先以低值(STATIONARY|FULL_SCREEN_AUXILIARY)
//   上屏，orderFront 之后的下一个 runloop tick 再写 273（0→1 转变才触发
//   WindowServer 重新注册贴附；同 tick 连写会被合并成 no-op）
// - orderFrontRegardless（不能 show()/makeKey，否则 AeroSpace 切 workspace）
// - 鼠标穿透：可交互状态（recording/transcribing/polishing）关穿透，其余保持穿透
// - 定位：鼠标所在显示器底部居中（visual_height + 80 底部留白，原版 bottom_visual_position）

import AppKit
import SwiftUI

final class CapsulePanelController: NSObject {
    static let shared = CapsulePanelController()

    @MainActor private let model = CapsuleModel()
    private var panel: NSPanel!
    /// classic pill 窗口几何：pill 42 + badge 区 28 + partial 区 40 + padding。
    /// visual_height 故意小于窗口高（下沉 40px，视觉中心回落，原版 capsule_visual_height）。
    private let windowSize = NSSize(width: 260, height: 130)

    override init() {
        super.init()
        let panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: windowSize),
            styleMask: [.nonactivatingPanel, .borderless],
            backing: .buffered,
            defer: false
        )
        panel.level = NSWindow.Level(rawValue: 25)
        panel.collectionBehavior = [.stationary, .fullScreenAuxiliary] // 入场帧低值
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false
        panel.ignoresMouseEvents = false
        panel.isReleasedWhenClosed = false
        self.panel = panel

        let model = self.model
        panel.contentView = NSHostingView(rootView: CapsuleHostView(model: model))

        // 事件路由：capsule:* → 胶囊状态机。
        EventSink.shared.addHandler { [weak self] event, payload in
            guard let self else { return }
            self.onCoreEvent(event: event, payload: payload)
        }
    }

    // MARK: - 事件

    @MainActor
    private func onCoreEvent(event: String, payload: String) {
        switch event {
        case "capsule:state":
            model.applyState(payload)
            syncPanel()
            // 录音提示音：进入 recording 播「叮咚」，离开 recording 停（原版 AudioCue）。
            // audio_cue_on_record 开关（P1.4 设置页）关闭时不播。
            if model.state == "recording" {
                if SettingsModel.shared.audioCueOnRecord {
                    AudioCue.playRecordStart()
                }
            } else {
                AudioCue.stop()
            }
        case "partial-text":
            model.applyPartial(payload)
        default:
            break
        }
    }

    /// 状态变化 → 同步面板显示/隐藏/穿透/位置。
    @MainActor
    private func syncPanel() {
        let interactive = model.interactive
        panel.ignoresMouseEvents = !interactive
        panel.isMovableByWindowBackground = false

        switch model.phase {
        case .entering, .visible:
            ensureVisible()
        case .leaving:
            // capsule-out 360ms（EXIT_ANIM_MS_CLASSIC）播完再隐藏。
            let model = self.model
            let panel = self.panel
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.36) {
                guard model.phase == .leaving else { return }
                panel?.orderOut(nil)
                model.phase = .hidden
            }
        case .hidden:
            break
        }
    }

    // MARK: - 显示/定位

    @MainActor
    private func ensureVisible() {
        if !panel.isVisible {
            position()
            panel.collectionBehavior = [.stationary, .fullScreenAuxiliary]
            panel.orderFrontRegardless()
            // 下个 runloop tick 写全值 273。
            DispatchQueue.main.async {
                self.panel.collectionBehavior = [
                    .canJoinAllSpaces, .stationary, .fullScreenAuxiliary,
                ]
            }
        }
    }

    /// 鼠标所在显示器底部居中。pill 底边距屏底 80px（原版 bottom_padding），
    /// pill 在窗口内居中 → 窗口左下角 y = 80 + pill半高21 - 窗口半高65。
    /// 注意：setFrameOrigin 是「窗口左下角」（原版 Tauri y 是窗口顶边，
    /// 直接套原版公式会把窗口放顶部——坐标语义不同）。
    @MainActor
    private func position() {
        let mouse = NSEvent.mouseLocation // 全局，左下原点
        let screen = NSScreen.screens.first { $0.frame.contains(mouse) }
            ?? NSScreen.main
            ?? NSScreen.screens.first
        guard let frame = screen?.frame else { return }
        let x = frame.minX + (frame.width - windowSize.width) / 2
        let y = frame.minY + 80 + 21 - windowSize.height / 2
        NSLog("[Capsule] position: mouse=%@ 屏 frame=%@ 原点=(%.0f,%.0f)", NSStringFromPoint(mouse), NSStringFromRect(frame), x, y)
        panel.setFrameOrigin(NSPoint(x: x.rounded(), y: y.rounded()))
    }
}
