// 主窗口壳 — 对齐原版 FloatingShell：188px 侧栏 + 内容区。
// - 侧栏：红绿灯 30px 预留（macOS）→ brand（AppIcon 22px + 「准记」）→ 9 项导航
// - 导航：13px/500，active = ink + 600 字重 + surface-2 圆角底（radius 8），hover 同底
// - 页面切换：原版 ol-page-slide（+10px 滑入 + fade 0.16s，移除 -6px fade）
// - 分隔：侧栏右缘 0.5px --ol-line

import SwiftUI

enum AppTab: String, CaseIterable, Identifiable {
    case overview, history, providers, dictionary
    case general, privacy, advanced

    var id: String { rawValue }

    /// 原版 i18n/zh-CN.ts nav.*（P1 文案写死中文）。
    var title: String {
        switch self {
        case .overview: "概览"
        case .history: "历史"
        case .providers: "ASR"
        case .dictionary: "词典"
        case .general: "通用"
        case .privacy: "隐私"
        case .advanced: "高级"
        }
    }

    /// 原版 Icon.tsx 1.5px stroke 路径 → SF Symbols 近似（line 风格）。
    var icon: String {
        switch self {
        case .overview: "chart.bar"
        case .history: "clock"
        case .providers: "cloud"
        case .dictionary: "book"
        case .general: "gearshape"
        case .privacy: "shield"
        case .advanced: "bolt"
        }
    }

    @ViewBuilder
    func page() -> some View {
        switch self {
        case .overview: OverviewView()
        case .history: HistoryView()
        case .providers: ProvidersView()
        case .dictionary: DictionaryView()
        case .general: SettingsGeneralView()
        case .privacy: SettingsPrivacyView()
        case .advanced: AdvancedView()
        }
    }
}

struct AppShellView: View {
    @State private var selection: AppTab = .overview

    var body: some View {
        HStack(spacing: 0) {
            SidebarView(selection: $selection)
            ZStack {
                Group {
                    // 「全部记录 →」切历史页（原版 onOpenHistory）。
                    if selection == .overview {
                        OverviewView(onOpenHistory: { selection = .history })
                    } else {
                        selection.page()
                    }
                }
                .id(selection)
                .transition(.asymmetric(
                    insertion: .offset(x: 10).combined(with: .opacity),
                    removal: .offset(x: -6).combined(with: .opacity)
                ))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.zhShellBG)
            .animation(.easeOut(duration: 0.16), value: selection)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

// MARK: - 侧栏

private struct SidebarView: View {
    @Binding var selection: AppTab

    var body: some View {
        VStack(spacing: 0) {
            // macOS 红绿灯悬浮区：侧栏与内容块顶到窗口最上沿，原生红绿灯浮在块面上。
            Color.clear.frame(height: 30)

            // brand — 原版 padding "2px 8px 12px"（顶 2 / 左右 8 / 底 12）。
            HStack(spacing: 9) {
                Image(nsImage: NSImage(named: "AppIcon") ?? NSImage())
                    .resizable()
                    .frame(width: 22, height: 22)
                    .clipShape(RoundedRectangle(cornerRadius: 5, style: .continuous))
                    .shadow(color: .black.opacity(0.1), radius: 1, y: 1)
                    .overlay(
                        RoundedRectangle(cornerRadius: 5, style: .continuous)
                            .stroke(Color.black.opacity(0.06), lineWidth: 0.5)
                    )
                Text("准记")
                    .font(.system(size: 13.5, weight: .semibold))
                    .kerning(-0.2)
                    .foregroundStyle(Color.zhInk)
                    .lineLimit(1)
                Spacer()
            }
            .padding(.top, 2)
            .padding(.horizontal, 8)
            .padding(.bottom, 12)

            // nav
            VStack(spacing: 1) {
                ForEach(AppTab.allCases) { tab in
                    SidebarButton(tab: tab, isSelected: selection == tab) {
                        selection = tab
                    }
                }
            }

            Spacer()

            // 侧栏底部版本号：从 Bundle 读，随打包版本自动更新。
            // padding leading 10 与 nav 按钮图标左缘对齐（aside 10 + 按钮 10）。
            if let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String {
                Text("v\(version)")
                    .font(.system(size: 11))
                    .foregroundStyle(Color.zhInk4)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.leading, 10)
            }
        }
        // aside 左右 10px + 底部 12px（原版 padding "30px 10px 12px"，顶部 30 由红绿灯区承担）。
        .padding(.horizontal, 10)
        .padding(.bottom, 12)
        .frame(width: 188)
        .background(Color.zhSidebarBG)
        .overlay(alignment: .trailing) {
            Rectangle().fill(Color.zhLine).frame(width: 0.5)
        }
    }
}

private struct SidebarButton: View {
    let tab: AppTab
    let isSelected: Bool
    let action: () -> Void

    @State private var hovered = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                // 原版 Icon size=14、stroke 恒定 1.5（不随选中变粗，仅文字变粗）。
                // 各 SF Symbol 固有尺寸不一，frame 统一 14×14 保证布局一致；
                // offset(y:1) 修正 symbol 光学中心偏高（对齐原版 flex center 的视觉中心）。
                Image(systemName: tab.icon)
                    .font(.system(size: 14))
                    .frame(width: 14, height: 14)
                    .offset(y: 1)
                Text(tab.title)
                    .font(.system(size: 12))
                Spacer()
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        // 原版 .ol-nav-btn：常态 ink-3/500，active ink/600 + surface-2 圆角底；
        // hover（非 active）同 active 底色。transition 0.16s（原版 navBtnStyle）。
        .foregroundStyle(isSelected ? Color.zhInk : Color.zhInk3)
        .fontWeight(isSelected ? .semibold : .medium)
        .background((isSelected || hovered) ? Color.zhSurface2 : Color.clear)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .animation(.easeOut(duration: 0.16), value: hovered)
        .animation(.easeOut(duration: 0.16), value: isSelected)
        .onHover { hovered = $0 }
    }
}
