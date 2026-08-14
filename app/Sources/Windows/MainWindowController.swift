// 主窗口（对齐原版 tauri main window）：
// 1240×800 / min 980×640 / hiddenTitle（红绿灯悬浮）/ close → 隐藏不退出。
// 透明圆角壳（ol-shell-radius: 32px）属 P3 视觉打磨，P1 先标准矩形窗口 + 全尺寸内容区。

import AppKit
import SwiftUI

final class MainWindowController: NSWindowController, NSWindowDelegate {
    static let shared = MainWindowController()

    private init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1240, height: 800),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "准记"
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.minSize = NSSize(width: 980, height: 640)
        window.isReleasedWhenClosed = false
        window.center()
        super.init(window: window)
        window.delegate = self
        // 权限 gate：不全时显示引导页（原版 App.tsx gate="onboarding"），
        // 授权完成后窗口激活自动切回主界面。
        window.contentView = NSHostingView(rootView: PermissionGateView())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not supported")
    }

    /// 前台显示（Dock 点击 / 菜单栏「打开准记」/ 权限引导共用）。
    func showMain() {
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }

    /// 关闭按钮 = 隐藏（原版 close → hide，从 Dock/菜单栏重新唤起）。
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        sender.orderOut(nil)
        return false
    }
}
