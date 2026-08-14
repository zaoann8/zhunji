// 准记（Zhunji）— 单进程 SwiftUI 原生版。
// 启动序列：EventSink 注册回调 → zhunji_init（主线程）→ core 发 app:core-ready →
// 显示主窗口（Dock 图标点击随时唤起；关闭窗口 = 隐藏，从 Dock/菜单栏重新打开）。

import AppKit
import SwiftUI

@main
struct ZhunjiApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        MenuBarExtra {
            VStack(alignment: .leading, spacing: 8) {
                Text("准记").font(.headline)
                Text("按住热键说话，自动转写并插入").font(.caption).foregroundStyle(.secondary)
                Divider()
                Button("打开准记") {
                    MainWindowController.shared.showMain()
                }
                Divider()
                Button("退出准记") {
                    NSApp.terminate(nil)
                }
            }
            .padding()
            .frame(width: 240)
        } label: {
            Image(nsImage: Self.trayIcon)
        }
        .menuBarExtraStyle(.window)
    }

    /// 菜单栏图标 = AppIcon（彩色，对齐原版 tray icon_as_template(false)）。
    private static let trayIcon: NSImage = {
        let fallback = NSImage(systemSymbolName: "mic.fill", accessibilityDescription: "准记")!
        // Bundle.main.url 比 NSImage(named:) 可靠（named 对无扩展名匹配有限）。
        guard let url = Bundle.main.url(forResource: "AppIcon", withExtension: "png"),
              let image = NSImage(contentsOf: url)
        else {
            NSLog("[Zhunji] AppIcon 未找到，菜单栏回退系统 mic 图标")
            return fallback
        }
        image.size = NSSize(width: 18, height: 18)
        image.isTemplate = false
        return image
    }()
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // 顺序不可换：先注册回调再 init（core-ready 事件才能收到）。
        EventSink.shared.register()
        // 触发 CapsulePanelController 懒加载：其 init 里注册 EventSink.handler
        // （capsule:* 事件 → 胶囊状态机），不初始化则 handler 为 nil、事件被丢弃。
        _ = CapsulePanelController.shared
        let code = zhunji_init()
        NSLog("[Zhunji] zhunji_init -> %d", code)
        // 设置页数据：prefs + 麦克风列表 + Grok 凭据（P1.4；胶囊提示音开关读它）。
        SettingsModel.shared.load()
        // 麦克风设备变化（插拔）→ 刷新设置页下拉。
        EventSink.shared.addHandler { event, payload in
            switch event {
            case "device:changed":
                SettingsModel.shared.loadMicrophones()
            case "microphone:level":
                // 设置页麦克风下拉的电平条（payload 为 {"level":f32}）。
                if let data = payload.data(using: .utf8),
                   let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let level = obj["level"] as? Double {
                    SettingsModel.shared.micLevel = level
                }
            case "network:result":
                if let data = payload.data(using: .utf8),
                   let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                    SettingsModel.shared.applyNetworkResult(obj)
                }
            case "history:changed":
                // 识别 / 删除 / 清空后概览页与历史页实时刷新（原版双页监听同一事件）。
                OverviewModel.shared.refresh()
                HistoryModel.shared.refresh()
            case "history:retranscribed":
                // 重转录完成：整条记录局部替换（原版 setItems(map)）。
                if let data = payload.data(using: .utf8),
                   let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                    HistoryModel.shared.applyRetranscribed(obj)
                }
            case "prefs:changed":
                // 偏好或供应商默认变化 → 设置页与供应商页双端同步刷新
                //（原版 setSettings/savePrefs 触发 settings 事件全局联动）。
                SettingsModel.shared.load()
                ProvidersModel.shared.refresh()
            case "provider:test-result":
                // 供应商页测试连通性结果（原版 test_provider 完成回调）。
                if let data = payload.data(using: .utf8),
                   let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                    ProvidersModel.shared.applyTestResult(obj)
                }
            case "engine:test-result":
                if let data = payload.data(using: .utf8),
                   let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                    OverviewModel.shared.applyTestResult(obj)
                }
            case "capsule:state":
                // 原版概览页监听 capsule:state：done → 引擎正常，error → 引擎错误。
                if let data = payload.data(using: .utf8),
                   let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                    OverviewModel.shared.applyCapsuleState(obj)
                }
            default:
                break
            }
        }
        // 启动显示主窗口（原版默认行为）；startMinimized = 静默启动：
        // 不弹主窗口，仅菜单栏运行（Dock 点击仍可唤起）。
        if !SettingsModel.shared.startMinimized {
            MainWindowController.shared.showMain()
        }
    }

    // Dock 图标点击 → 显示主窗口（窗口被隐藏时也能唤起）。
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        MainWindowController.shared.showMain()
        return false
    }

    func applicationWillTerminate(_ notification: Notification) {
        zhunji_request_shutdown()
    }
}
