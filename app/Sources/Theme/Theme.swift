// 主题色板 — 对照原版 zhunlu/src/styles/tokens.css 的 --ol-* token。
// 用 NSColor dynamic provider 实现随系统深浅色自适应，SwiftUI 侧统一走 Color(nsColor:)。
// 值直接取自原版 tokens（浅色段 95-125 行，深色段 266-286 行附近）。

import AppKit
import SwiftUI

extension NSColor {
    /// hex 0xRRGGBB → NSColor（sRGB）。
    convenience init(hex: UInt32) {
        let r = CGFloat((hex >> 16) & 0xFF) / 255.0
        let g = CGFloat((hex >> 8) & 0xFF) / 255.0
        let b = CGFloat(hex & 0xFF) / 255.0
        self.init(srgbRed: r, green: g, blue: b, alpha: 1)
    }

    private static func zh(_ light: UInt32, _ dark: UInt32) -> NSColor {
        NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(hex: isDark ? dark : light)
        }
    }

    /// 带透明度的动态色：参数 (RGB, alpha)，原版 rgba() 的直接移植。
    private static func zh(_ light: (UInt32, CGFloat), _ dark: (UInt32, CGFloat)) -> NSColor {
        NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            let (rgb, alpha) = isDark ? dark : light
            return NSColor(hex: rgb).withAlphaComponent(alpha)
        }
    }

    // --ol-ink / --ol-ink-3（导航文字，active/常态）
    static let zhInk = zh(0x09090B, 0xFAFAFA)
    static let zhInk3 = zh(0x71717A, 0xA1A1AA)

    // --ol-surface-2（nav active/hover 底）、--ol-line（0.5px 分隔线）
    static let zhSurface2 = zh(0xF4F4F5, 0x2A2A2E)
    static let zhLine = zh(0xE4E4E7, 0x27272A)

    // --ol-sidebar-bg（侧栏底）与 --ol-app-shell-bg（内容底，≈ surface）
    static let zhSidebarBG = zh(0xFAFAFA, 0x141417)
    static let zhShellBG = zh(0xFFFFFF, 0x1C1C1F)

    // --ol-blue（accent，胶囊条/链接等）
    static let zhBlue = zh(0x2563EB, 0x74B7FF)

    // --ol-ink-2 / --ol-err（胶囊 thinking 扫光文字底、error 文案）
    static let zhInk2 = zh(0x3F3F46, 0xD4D4D8)
    static let zhErr = zh(0xDC2626, 0xF87171)

    // 权限引导页（Onboarding.tsx）用到的其余 token：
    // --ol-surface（卡片底）/ --ol-line-soft（步骤分隔）/ --ol-line-strong（按钮描边）
    // / --ol-ink-4（hint 弱文字）/ --ol-ok（已授权徽章绿）
    // / --ol-primary-solid（主按钮：浅=黑底白字，深=蓝底白字，tokens.css 9-10/215-216 行）
    static let zhSurface = zh(0xFFFFFF, 0x1C1C1F)
    static let zhLineSoft = zh(0xF4F4F5, 0x1F1F23)
    static let zhLineStrong = zh(0xD4D4D8, 0x3F3F46)
    static let zhInk4 = zh(0xA1A1AA, 0x71717A)
    static let zhOK = zh(0x16A34A, 0x4ADE80)
    static let zhOKSoft = zh(0xECFDF5, 0x1C2A22)
    static let zhPrimarySolidBG = zh(0x09090B, 0x3B82F6)
    static let zhPrimarySolidInk = zh(0xFFFFFF, 0xF8FBFF)

    // --ol-capsule-*（胶囊 pill / 按钮 / badge，原版 tokens.css 197-208 行 + 深色段 362-374 行）
    static let zhPillBG = zh((0xFFFFFF, 0.90), (0x1B222C, 1.0))
    static let zhPillBorder = zh((0x000000, 0.10), (0xE2E8F0, 0.12))
    static let zhPillInset = zh((0xFFFFFF, 0.72), (0xFFFFFF, 0.08))
    static let zhBtnBG = zh((0x000000, 0.06), (0x26313F, 1.0))
    static let zhBtnBGConfirm = zh((0x000000, 0.08), (0x2B3848, 1.0))
    static let zhBtnInk = zh((0x2A2A2D, 1.0), (0xF4F7FB, 1.0))
    static let zhBtnBorder = zh((0x000000, 0.10), (0xE2E8F0, 0.14))
    static let zhCenterInk = zh((0x0A0A0B, 0.72), (0xF4F7FB, 0.72))
    static let zhBadgeBG = zh((0xFFFFFF, 0.82), (0x1B222C, 0.94))
    static let zhBadgeBorder = zh((0x2563EB, 0.25), (0x74B7FF, 0.30))

    // MARK: 设置页 token（原版 tokens.css：Segmented / Toggle / Select / Pill）

    /// --ol-blue-soft：本地优先说明条底。
    static let zhBlueSoft = zh(0xEFF4FF, 0x1C2A42)
    /// --ol-segmented-bg：录音方式分段控件轨道。
    static let zhSegmentedBG = zh((0x000000, 0.04), (0xE2E8F0, 0.07))
    /// --ol-segmented-active-bg（浅色 #ffffff 实底，深色取渐变近似 #3f3f46）。
    static let zhSegmentedActiveBG = zh(0xFFFFFF, 0x3F3F46)
    /// --ol-segmented-active-shadow。
    static let zhSegmentedActiveShadow = zh((0x000000, 0.08), (0x000000, 0.36))
    /// --ol-toggle-off-bg / --ol-toggle-knob。
    static let zhToggleOffBG = zh((0x000000, 0.15), (0xE2E8F0, 0.18))
    static let zhToggleKnob = zh(0xFFFFFF, 0xFAFAFA)
    /// --ol-select-popover-bg / --ol-select-option-hover-bg。
    static let zhSelectPopoverBG = zh((0xFFFFFF, 0.97), (0x1C1C1F, 0.98))
    static let zhSelectOptionHover = zh((0x2563EB, 0.10), (0x74B7FF, 0.15))
    /// --ol-pill-bg（default tone，浅色渐变 ≈ #f7f7f9 近似）/ --ol-pill-ok-bg / --ol-pill-blue-bg。
    static let zhPillDefaultBG = zh((0xF7F7F9, 1.0), (0xE2E8F0, 0.07))
    static let zhPillOKBG = zh((0x4ADE80, 0.14), (0x4ADE80, 0.16))
    static let zhPillBlueBG = zh((0x60A5FA, 0.14), (0x74B7FF, 0.16))
    /// --ol-pill-selected-*（历史页筛选 chip 选中：浅色黑底白字，深色蓝渐变）。
    static let zhPillSelectedBG = zh(0x18181B, 0x3B82F6)
    static let zhPillSelectedInk = zh(0xFFFFFF, 0xF4F7FB)
    static let zhPillSelectedBorder = zh(0x18181B, 0x93C5FD)
}

extension Color {
    /// 便捷构造：SwiftUI 侧直接 Color(nsColor: .zhInk) 亦可，这里给常用名缩写。
    static let zhInk = Color(nsColor: .zhInk)
    static let zhInk3 = Color(nsColor: .zhInk3)
    static let zhSurface2 = Color(nsColor: .zhSurface2)
    static let zhLine = Color(nsColor: .zhLine)
    static let zhSidebarBG = Color(nsColor: .zhSidebarBG)
    static let zhShellBG = Color(nsColor: .zhShellBG)
    static let zhBlue = Color(nsColor: .zhBlue)
    static let zhInk2 = Color(nsColor: .zhInk2)
    static let zhErr = Color(nsColor: .zhErr)
    static let zhPillBG = Color(nsColor: .zhPillBG)
    static let zhPillBorder = Color(nsColor: .zhPillBorder)
    static let zhBtnBG = Color(nsColor: .zhBtnBG)
    static let zhBtnBGConfirm = Color(nsColor: .zhBtnBGConfirm)
    static let zhBtnInk = Color(nsColor: .zhBtnInk)
    static let zhBtnBorder = Color(nsColor: .zhBtnBorder)
    static let zhCenterInk = Color(nsColor: .zhCenterInk)
    static let zhBadgeBG = Color(nsColor: .zhBadgeBG)
    static let zhBadgeBorder = Color(nsColor: .zhBadgeBorder)
    static let zhSurface = Color(nsColor: .zhSurface)
    static let zhLineSoft = Color(nsColor: .zhLineSoft)
    static let zhLineStrong = Color(nsColor: .zhLineStrong)
    static let zhInk4 = Color(nsColor: .zhInk4)
    static let zhOK = Color(nsColor: .zhOK)
    static let zhOKSoft = Color(nsColor: .zhOKSoft)
    static let zhPrimarySolidBG = Color(nsColor: .zhPrimarySolidBG)
    static let zhPrimarySolidInk = Color(nsColor: .zhPrimarySolidInk)
    static let zhBlueSoft = Color(nsColor: .zhBlueSoft)
    static let zhSegmentedBG = Color(nsColor: .zhSegmentedBG)
    static let zhSegmentedActiveBG = Color(nsColor: .zhSegmentedActiveBG)
    static let zhSegmentedActiveShadow = Color(nsColor: .zhSegmentedActiveShadow)
    static let zhToggleOffBG = Color(nsColor: .zhToggleOffBG)
    static let zhToggleKnob = Color(nsColor: .zhToggleKnob)
    static let zhSelectPopoverBG = Color(nsColor: .zhSelectPopoverBG)
    static let zhSelectOptionHover = Color(nsColor: .zhSelectOptionHover)
    static let zhPillDefaultBG = Color(nsColor: .zhPillDefaultBG)
    static let zhPillOKBG = Color(nsColor: .zhPillOKBG)
    static let zhPillBlueBG = Color(nsColor: .zhPillBlueBG)
    static let zhPillSelectedBG = Color(nsColor: .zhPillSelectedBG)
    static let zhPillSelectedInk = Color(nsColor: .zhPillSelectedInk)
    static let zhPillSelectedBorder = Color(nsColor: .zhPillSelectedBorder)
}
