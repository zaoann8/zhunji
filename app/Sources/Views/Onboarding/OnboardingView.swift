// 权限引导页 — 移植自原版 Onboarding.tsx（zhunlu/src/components/Onboarding.tsx）：
// gate 模式：启动时权限不全 → 整窗口显示本页；授权后（窗口激活时自动刷新）
// 切回 AppShell。视觉 1:1 对照原版：居中卡片 520px / BrandHeader / 步骤行 /
// footerHint / 「仅进入设置」次级按钮。文案 = 原版 i18n/zh-CN.ts onboarding.*。
//
// 与原版的差异（P1 精简，标注于各方法）：
// - 无「重置授权并重启」（依赖 tauri 插件 tccutil reset，native 暂不提供）
// - accessibility 无 notDetermined 态（AXIsProcessTrusted 只返回 granted/denied）

import AppKit
import ApplicationServices
import AVFoundation
import SwiftUI

enum PermissionStatus {
    case granted, denied, notDetermined, noDevice
}

/// 权限状态 + gate 判定。窗口每次激活时 refresh（原版 window focus 事件）。
@MainActor
final class PermissionModel: ObservableObject {
    @Published var accessibility: PermissionStatus = .denied
    @Published var microphone: PermissionStatus = .notDetermined
    /// 用户点「仅进入设置」后不再阻塞（原版 continueToSettings）。
    @Published var skipRequested = false

    var needsOnboarding: Bool {
        if skipRequested { return false }
        let aOk = accessibility == .granted
        // noDevice = 没有麦克风设备，不是权限问题，不阻塞（原版注释同）。
        let mOk = microphone == .granted || microphone == .noDevice
        return !(aOk && mOk)
    }

    func refresh() {
        accessibility = AXIsProcessTrusted() ? .granted : .denied
        let status = AVCaptureDevice.authorizationStatus(for: .audio)
        switch status {
        case .authorized: microphone = .granted
        case .denied, .restricted: microphone = .denied
        default:
            let hasDevice = !AVCaptureDevice.DiscoverySession(
                deviceTypes: [.microphone], mediaType: .audio, position: .unspecified
            ).devices.isEmpty
            microphone = hasDevice ? .notDetermined : .noDevice
        }
    }

    /// 「授权」：弹 TCC 提示框（ad-hoc 开发构建可能不弹，macOS 15 已知行为）；
    /// 未授予则打开系统设置对应页（原版 requestAccessibilityPermission → denied → openSystemSettings）。
    func grantAccessibility() {
        let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        let granted = AXIsProcessTrustedWithOptions(options)
        refresh()
        // 原版 setTimeout(refresh, 800)：TCC 状态落库可能滞后，二次确认。
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { [weak self] in
            self?.refresh()
        }
        if !granted {
            NSWorkspace.shared.open(
                URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")!
            )
        }
    }

    /// 麦克风：未请求过 → requestAccess；被拒 → 打开系统设置麦克风页。
    /// 授权结果优先取 TCC 回调返回值（authorizationStatus 有读取时序，可能滞后）；
    /// 800ms 后二次 refresh 兜底（原版 setTimeout(refresh, 800)）。
    func requestMicrophone() {
        if microphone == .denied {
            NSWorkspace.shared.open(
                URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!
            )
            return
        }
        AVCaptureDevice.requestAccess(for: .audio) { [weak self] granted in
            DispatchQueue.main.async {
                guard let self else { return }
                self.refresh()
                // 回调 granted=true 但 authorizationStatus 未及更新时，以回调为准，
                // 保证引导页行状态立即同步（原版 setMicrophone(status) 直接置位）。
                if granted && self.microphone != .granted {
                    self.microphone = .granted
                }
                // 原版：TCC 落库滞后 → 800ms 后二次确认；仍为 denied 才开系统设置。
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { [weak self] in
                    guard let self else { return }
                    self.refresh()
                    if self.microphone == .denied {
                        NSWorkspace.shared.open(
                            URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!
                        )
                    }
                }
            }
        }
    }
}

/// gate：权限不全且未跳过 → 引导页，否则主界面。
struct PermissionGateView: View {
    @StateObject private var model = PermissionModel()
    @ObservedObject private var settings = SettingsModel.shared

    var body: some View {
        Group {
            if model.needsOnboarding {
                OnboardingView(model: model)
            } else {
                AppShellView()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // 顶到窗口最上沿：fullSizeContentView 下 safe area 仍含 titlebar 高度，
        // 不忽略会让整个界面下移一截（原版网页无 safe area、内容从 y=0 开始，
        // 红绿灯直接浮在侧栏表面）。
        .ignoresSafeArea()
        // 主题（原版 ThemeSection 的 themeMode 立即生效）。
        .preferredColorScheme(themeColorScheme)
        .onAppear { model.refresh() }
        .onReceive(NotificationCenter.default.publisher(
            for: NSApplication.didBecomeActiveNotification
        )) { _ in
            // 授权必经系统设置，切回本 app 时刷新（原版 focus/visibilitychange）。
            model.refresh()
        }
    }

    /// system → nil（跟随系统），light/dark 显式指定。
    private var themeColorScheme: ColorScheme? {
        switch settings.themeMode {
        case "light": return .light
        case "dark": return .dark
        default: return nil
        }
    }
}

// MARK: - 引导页

struct OnboardingView: View {
    @ObservedObject var model: PermissionModel

    var body: some View {
        VStack {
            Spacer(minLength: 0)
            card
            Spacer(minLength: 0)
        }
        .padding(18)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.zhShellBG)
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: 0) {
            BrandHeader()

            PermissionStep(
                index: 1,
                title: "辅助功能",
                desc: "用于监听全局快捷键（默认 右 Control）并把识别结果写入光标位置。",
                status: model.accessibility,
                actionLabel: model.accessibility == .granted ? "已授权" : "授权",
                disabled: model.accessibility == .granted,
                hint: "授权后必须完全退出准记再重新打开（macOS TCC 规则）。",
                onAction: model.grantAccessibility
            )

            PermissionStep(
                index: 2,
                title: "麦克风",
                desc: "用于捕获你的语音输入。",
                status: model.microphone,
                actionLabel: {
                    switch model.microphone {
                    case .granted: "已授权"
                    case .noDevice: "重试"
                    case .denied: "打开系统设置"
                    default: "弹出授权"
                    }
                }(),
                disabled: model.microphone == .granted,
                hint: model.microphone == .noDevice ? "未检测到麦克风，请连接并启用麦克风后重试。" : nil,
                onAction: model.microphone == .noDevice ? model.refresh : model.requestMicrophone
            )

            footerHint

            Button("仅进入设置（语音与全局快捷键暂不可用）") {
                model.skipRequested = true
            }
            .buttonStyle(.plain)
            .font(.system(size: 12.5, weight: .medium))
            .foregroundStyle(Color.zhInk2)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 9)
            .background(Color.zhSurface)
            .overlay(
                RoundedRectangle(cornerRadius: 8).strokeBorder(Color.zhLineStrong, lineWidth: 0.5)
            )
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .padding(.top, 12)
        }
        .padding(32)
        .frame(maxWidth: 520)
        .background(Color.zhSurface)
        .clipShape(RoundedRectangle(cornerRadius: 14))
        .overlay(
            RoundedRectangle(cornerRadius: 14).strokeBorder(Color.zhLine, lineWidth: 0.5)
        )
        .shadow(color: .black.opacity(0.09), radius: 24, x: 0, y: -6)
    }

    /// 原版 footerHint：surface-2 底 / 圆角 8 / 11.5px ink-3。
    private var footerHint: some View {
        Text("授权全部完成后此引导自动关闭。如果一直不消失，从菜单栏 准记 → 退出，重新打开 App。")
            .font(.system(size: 11.5))
            .foregroundStyle(Color.zhInk3)
            .lineSpacing(2.5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .background(Color.zhSurface2)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .padding(.top, 18)
    }
}

/// 品牌头：AppIcon 52×52 圆角 13 + 标题 18px/650 + 描述 12.5px ink-3。
private struct BrandHeader: View {
    var body: some View {
        HStack(spacing: 14) {
            Image(nsImage: NSImage(named: "AppIcon") ?? NSImage())
                .resizable()
                .frame(width: 52, height: 52)
                .clipShape(RoundedRectangle(cornerRadius: 13, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                Text("欢迎使用准记")
                    // 原版 fontWeight 650（NSFont.Weight 支持中间值，variable font 插值）。
                    .font(Font(NSFont.systemFont(
                        ofSize: 18, weight: NSFont.Weight(rawValue: 650))))
                    .foregroundStyle(Color.zhInk)
                Text("本地说出，本地落字。开始前需要两个系统权限。")
                    .font(.system(size: 12.5))
                    .foregroundStyle(Color.zhInk3)
                    .lineSpacing(1.5)
            }
            Spacer(minLength: 0)
        }
        .padding(.bottom, 18)
    }
}

/// 步骤行：22px 序号圆（授权后蓝底白✓）+ 标题/描述/hint + 右侧动作按钮。
/// 分隔线 borderTop 0.5px line-soft（原版用 borderTop 而非 Divider）。
private struct PermissionStep: View {
    let index: Int
    let title: String
    let desc: String
    let status: PermissionStatus
    let actionLabel: String
    let disabled: Bool
    var hint: String?
    let onAction: () -> Void

    private var granted: Bool { status == .granted }

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            // 序号/✓ 圆
            ZStack {
                Circle()
                    .fill(granted ? Color.zhOKSoft : Color.black.opacity(0.06))
                    .frame(width: 22, height: 22)
                if granted {
                    Image(systemName: "checkmark")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(Color.zhOK)
                } else {
                    Text("\(index)")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(Color.zhInk3)
                }
            }

            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.system(size: 13.5, weight: .semibold))
                    .foregroundStyle(Color.zhInk)
                Text(desc)
                    .font(.system(size: 12))
                    .foregroundStyle(Color.zhInk3)
                    .lineSpacing(1.5)
                if let hint {
                    // 原版 hint 里 **粗体** 段用 ink-2。P1 文案无加粗段，整体 ink-4。
                    Text(hint)
                        .font(.system(size: 11))
                        .foregroundStyle(Color.zhInk4)
                        .lineSpacing(1.5)
                        .padding(.top, 1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Button(action: onAction) {
                Text(actionLabel)
                    .font(.system(size: 12.5, weight: .medium))
                    .foregroundStyle(granted ? Color.zhInk3 : Color.zhPrimarySolidInk)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .background(granted ? Color.zhSurface2 : Color.zhPrimarySolidBG)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }
            .buttonStyle(.plain)
            .disabled(disabled)
            .opacity(disabled && !granted ? 0.6 : 1)
        }
        .padding(.vertical, 14)
        .overlay(alignment: .top) {
            Rectangle().fill(Color.zhLineSoft).frame(height: 0.5)
        }
    }
}
