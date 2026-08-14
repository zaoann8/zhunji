// 开机自启（原版 tauri-plugin-autostart · MacosLauncher::LaunchAgent）：
// 写 ~/Library/LaunchAgents/com.zhunji.app.plist + launchctl load/unload。
// 状态由 OS 持有（plist），不存进 prefs（issue #194：prefs 缓存会与 OS 真相不一致）。

import Foundation

enum Autostart {
    static let bundleId = "com.zhunji.app"

    private static var plistURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents/\(bundleId).plist")
    }

    /// 当前是否已注册（plist 存在且 launchctl 认识）。
    static func isEnabled() -> Bool {
        guard FileManager.default.fileExists(atPath: plistURL.path) else { return false }
        let output = run("/bin/launchctl", ["print", "gui/\(getuid())/\(bundleId)"])
        return output.0 == 0
    }

    /// 注册开机自启。失败返回错误描述（原版 toggle 失败在行内红字提示）。
    static func enable() throws {
        let plist = [
            "Label": bundleId,
            "ProgramArguments": [Bundle.main.bundleURL.path],
            "RunAtLoad": true,
        ] as [String: Any]
        let data = try PropertyListSerialization.data(
            fromPropertyList: plist,
            format: .xml,
            options: 0
        )
        try data.write(to: plistURL, options: .atomic)
        let (code, message) = run("/bin/launchctl", ["load", plistURL.path])
        guard code == 0 else {
            try? FileManager.default.removeItem(at: plistURL)
            throw AutostartError.message(message)
        }
    }

    /// 取消开机自启。
    static func disable() throws {
        let (code, message) = run("/bin/launchctl", ["unload", plistURL.path])
        if code != 0 && FileManager.default.fileExists(atPath: plistURL.path) {
            // plist 未加载也允许直接删（launchctl unload 对未加载文件同样成功，这里兜底）。
            throw AutostartError.message(message)
        }
        try? FileManager.default.removeItem(at: plistURL)
    }

    private static func run(_ executable: String, _ args: [String]) -> (Int32, String) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = args
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        do {
            try process.run()
        } catch {
            return (1, error.localizedDescription)
        }
        process.waitUntilExit()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        let text = String(data: data, encoding: .utf8) ?? ""
        return (process.terminationStatus, text.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    enum AutostartError: LocalizedError {
        case message(String)
        var errorDescription: String? {
            if case .message(let m) = self { return m }
            return nil
        }
    }
}
