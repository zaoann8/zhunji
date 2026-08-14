// 胶囊主视图 — 移植自原版 Capsule.tsx 的 ClassicCapsule 分支：
// ClassicPill（176×42）/ AudioBars / ShineText / CenterText / CircleButton /
// TranslatingBadge + capsule-in / capsule-out / cap-shine / cap-state-enter 动画。
// 所有尺寸、颜色、动画曲线、timing 直接取自原版（CLASSIC_PILL_METRICS、
// tokens.css --ol-capsule-* 浅/深两套），Swift 侧逐条对照实现。

import SwiftUI

/// 胶囊宿主：260×130 透明面板。pill 恒居中心；badge 锚定 pill 顶上方 8px；
/// partial text 落在 pill 下方（bottom ≈ 34，与原版 Siri 分支的 bottom:34 一致）。
/// 进出场动画由 phase 驱动：entering 播 capsule-in（0.38s），leaving 播
/// capsule-out（0.36s），时长与原版 EXIT_ANIM_MS_CLASSIC 同步（CapsulePanel
/// 的 orderOut 定时器依赖此值）。
struct CapsuleHostView: View {
    @ObservedObject var model: CapsuleModel
    /// capsule-in/out 的当前姿态（scale/translate/opacity 目标值）。
    @State private var shown = false

    var body: some View {
        ZStack {
            if model.phase != .hidden {
                ZStack(alignment: .center) {
                    PillView(model: model)
                    // badge：pill 中线上方 21+8px（pill 半高 + gap），底边到 pill 顶 8px。
                    TranslatingBadge(visible: model.translation)
                        .offset(y: -(21 + 8 + 9.5))
                    if model.state == "recording",
                       let text = model.partialText, !text.isEmpty
                    {
                        Text(text)
                            .font(.system(size: 12.5, weight: .medium))
                            .foregroundColor(Color(nsColor: .zhInk3))
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .frame(maxWidth: 230)
                            .offset(y: 21 + 6 + 8) // pill 半高 + gap + 文字半高 ≈ 34
                    }
                }
                .scaleEffect(shown ? 1 : 0.46)
                .offset(y: shown ? 0 : 18)
                .opacity(shown ? 1 : 0)
            }
        }
        .frame(width: 260, height: 130)
        .onChange(of: model.phase) { _, phase in
            switch phase {
            case .entering:
                // capsule-in .38s cubic-bezier(.3,1.2,.4,1)：曲线 y>1 自带
                // scale 1.035 overshoot + translateY(-1) 回弹，无需显式 keyframe。
                withAnimation(.timingCurve(0.3, 1.2, 0.4, 1, duration: 0.38)) {
                    shown = true
                }
            case .leaving:
                // capsule-out .36s cubic-bezier(.55,.06,.68,.19)。
                withAnimation(.timingCurve(0.55, 0.06, 0.68, 0.19, duration: 0.36)) {
                    shown = false
                }
            default:
                break
            }
        }
    }
}

/// 经典药丸：176×42，HStack cancel|center|confirm（gap 4，padding 0 8px）。
/// ambient（=level，仅 recording）驱动 scale 1+ambient*0.018 与外层阴影 alpha。
private struct PillView: View {
    @ObservedObject var model: CapsuleModel

    private var cancelEnabled: Bool {
        model.state == "recording" || model.state == "transcribing" || model.state == "polishing"
    }
    private var confirmEnabled: Bool { model.state == "recording" }
    private var ambient: Double {
        model.state == "recording" ? min(1, max(0, model.level)) : 0
    }

    var body: some View {
        HStack(spacing: 4) {
            CircleButton(variant: .cancel, enabled: cancelEnabled) {
                zhunji_capsule_cancel()
            }
            PillCenter(model: model)
                .frame(maxWidth: .infinity)
            CircleButton(variant: .confirm, enabled: confirmEnabled) {
                zhunji_capsule_confirm()
            }
        }
        .padding(.horizontal, 8)
        .frame(width: 176, height: 42)
        .background(
            // 背景层单独投射阴影，避免文字/图标也被投影；顶部 1px 内高光
            // （--ol-capsule-pill-inset）随背景一起 clip，圆角处不外露。
            ZStack {
                Color(nsColor: .zhPillBG)
                VStack {
                    Color(nsColor: .zhPillInset).frame(height: 1)
                    Spacer(minLength: 0)
                }
            }
            .clipShape(Capsule())
            .shadow(
                color: .black.opacity(0.2 + ambient * 0.1),
                radius: 40, x: 0, y: 18 // 原版 -10px spread 并入 radius
            )
        )
        .overlay(Capsule().strokeBorder(.black.opacity(0.24), lineWidth: 0.5))
        .overlay(Capsule().strokeBorder(Color(nsColor: .zhPillBorder), lineWidth: 1))
        .scaleEffect(1 + ambient * 0.018)
        .animation(.easeOut(duration: 0.08), value: ambient)
    }
}

/// pill 中心内容：录音电平条 / thinking 扫光 / 完成文案，按 state 切换。
private struct PillCenter: View {
    @ObservedObject var model: CapsuleModel
    /// cap-state-enter：进入 processing 时 220ms 淡入 + 2px 上移；切走瞬间消失。
    @State private var thinkingVisible = false

    var body: some View {
        ZStack {
            switch model.state {
            case "recording":
                AudioBarsView(level: model.level)
            case "transcribing", "polishing":
                ShineText(period: model.shineFast ? 0.9 : 2.4)
                    .padding(.horizontal, 4)
                    .opacity(thinkingVisible ? 1 : 0)
                    .offset(y: thinkingVisible ? 0 : 2)
            case "done":
                CenterText(text: model.message ?? "已插入 \(model.insertedChars ?? 0)")
            case "cancelled":
                CenterText(text: "已取消")
            case "error":
                CenterText(text: model.message ?? "出错了", color: Color(nsColor: .zhErr))
            default:
                AudioBarsView(level: 0)
            }
        }
        .onChange(of: model.state) { _, newState in
            if newState == "transcribing" || newState == "polishing" {
                withAnimation(.easeOut(duration: 0.22)) { thinkingVisible = true }
            } else {
                thinkingVisible = false
            }
        }
    }
}

/// 录音电平条：5 条，envelope [0.55,0.85,1.0,0.85,0.55]。
/// 电平数学逐字对照原版：silenceGate 0.012 / responseCeiling 0.34 → smoothstep
/// (x²(3-2x)) → pow 0.42；height 2→24，0.18s cubic-bezier(0.22,1,0.36,1)。
struct AudioBarsView: View {
    let level: Double

    private static let envelope: [Double] = [0.55, 0.85, 1.0, 0.85, 0.55]
    private let base: Double = 2
    private let maxHeight: Double = 24

    private func barHeight(_ env: Double) -> CGFloat {
        let voice = min(1, max(0, level))
        let silenceGate = 0.012
        let responseCeiling = 0.34
        let gated = min(1, max(0, (voice - silenceGate) / (responseCeiling - silenceGate)))
        let eased = gated * gated * (3 - 2 * gated)
        let visual = pow(eased, 0.42)
        return base + (maxHeight - base) * visual * env
    }

    var body: some View {
        HStack(spacing: 3) {
            ForEach(Array(Self.envelope.enumerated()), id: \.offset) { _, env in
                Capsule()
                    .fill(Color(nsColor: .zhBlue).opacity(0.82))
                    .frame(width: 3, height: barHeight(env))
                    .animation(
                        .timingCurve(0.22, 1, 0.36, 1, duration: 0.18),
                        value: level
                    )
            }
        }
        .frame(width: 42, height: 24, alignment: .center)
    }
}

/// "thinking" 蓝光扫过文字：ink-2 底 + blue 扫光（cap-shine keyframe）。
/// 原版 linear-gradient(100deg) + background-size 220% + position 200%→-200%，
/// 移植为 5-stop LinearGradient 在文本宽度 2.2 倍区间内平移。
struct ShineText: View {
    /// 扫光周期（秒）：burst 0.9 / 稳态 2.4。
    let period: Double

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 60.0)) { context in
            let cycle = context.date.timeIntervalSinceReferenceDate / period
            let t = cycle - floor(cycle)
            let pos = 2.0 - 4.0 * t // 200% → -200%
            Text("thinking")
                .font(.system(size: 17, weight: .semibold))
                .tracking(0.3)
                .foregroundStyle(LinearGradient(
                    stops: [
                        .init(color: Color(nsColor: .zhInk2), location: 0.0),
                        .init(color: Color(nsColor: .zhInk2), location: 0.35),
                        .init(color: Color(nsColor: .zhBlue), location: 0.5),
                        .init(color: Color(nsColor: .zhInk2), location: 0.65),
                        .init(color: Color(nsColor: .zhInk2), location: 1.0),
                    ],
                    startPoint: .init(x: pos, y: 0.5),
                    endPoint: .init(x: pos + 2.2, y: 0.5)
                ))
        }
        .frame(maxWidth: 84)
    }
}

/// 状态文案：11px/500，maxWidth 84，超长省略（原版 CenterText）。
struct CenterText: View {
    let text: String
    var color: Color = Color(nsColor: .zhCenterInk)

    var body: some View {
        Text(text)
            .font(.system(size: 11, weight: .medium))
            .foregroundColor(color)
            .lineLimit(1)
            .truncationMode(.tail)
            .frame(maxWidth: 84)
    }
}

/// 「正在翻译」徽章：dot 5px + 10.5px/600 蓝字，锚定 pill 顶上方 8px。
/// 出现：translateY(40px) scale(.88) → 归位，opacity .24s ease-out +
/// transform .34s cubic-bezier(.2,.9,.3,1.1)（两条曲线分别驱动，不合并）。
struct TranslatingBadge: View {
    let visible: Bool
    @State private var opacityShown = false
    @State private var transformShown = false

    var body: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(Color(nsColor: .zhBlue))
                .frame(width: 5, height: 5)
            Text("正在翻译")
                .font(.system(size: 10.5, weight: .semibold))
                .tracking(0.2)
                .foregroundColor(Color(nsColor: .zhBlue))
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 3)
        .background(Capsule().fill(Color(nsColor: .zhBadgeBG)))
        .overlay(Capsule().strokeBorder(Color(nsColor: .zhBadgeBorder), lineWidth: 0.5))
        .shadow(color: Color(red: 37 / 255, green: 99 / 255, blue: 235 / 255).opacity(0.25), radius: 12, x: 0, y: 4)
        .shadow(color: .black.opacity(0.04), radius: 0.5)
        .opacity(opacityShown ? 1 : 0)
        .scaleEffect(transformShown ? 1 : 0.88, anchor: .bottom)
        .offset(y: transformShown ? 0 : 40)
        .onAppear {
            opacityShown = visible
            transformShown = visible
        }
        .onChange(of: visible) { _, newValue in
            if newValue {
                withAnimation(.easeOut(duration: 0.24)) { opacityShown = true }
                withAnimation(.timingCurve(0.2, 0.9, 0.3, 1.1, duration: 0.34)) {
                    transformShown = true
                }
            } else {
                withAnimation(.easeOut(duration: 0.24)) { opacityShown = false }
                withAnimation(.timingCurve(0.2, 0.9, 0.3, 1.1, duration: 0.34)) {
                    transformShown = false
                }
            }
        }
    }
}

/// 28×28 圆形按钮（✗ / ✓），描边 0.8px + 投影 0 1px 2px。disabled → opacity 0.42。
struct CircleButton: View {
    enum Variant {
        case cancel, confirm
    }

    let variant: Variant
    let enabled: Bool
    let action: () -> Void

    private var bg: Color {
        variant == .cancel
            ? Color(nsColor: .zhBtnBG)
            : Color(nsColor: .zhBtnBGConfirm)
    }

    var body: some View {
        Button(action: action) {
            ZStack {
                Circle()
                    .fill(bg)
                    .shadow(color: .black.opacity(0.06), radius: 2, x: 0, y: 1)
                Circle()
                    .strokeBorder(Color(nsColor: .zhBtnBorder), lineWidth: 0.8)
                icon
                    .foregroundColor(Color(nsColor: .zhBtnInk))
            }
            .frame(width: 28, height: 28)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.42)
        .animation(.easeOut(duration: 0.18), value: enabled)
    }

    /// 原版内联 SVG path（11×11 ✗ stroke 1.6 / 13×13 ✓ stroke 1.7）。
    private var icon: some View {
        Group {
            if variant == .cancel {
                Path { p in
                    p.move(to: CGPoint(x: 1.5, y: 1.5))
                    p.addLine(to: CGPoint(x: 9.5, y: 9.5))
                    p.move(to: CGPoint(x: 9.5, y: 1.5))
                    p.addLine(to: CGPoint(x: 1.5, y: 9.5))
                }
                .stroke(style: StrokeStyle(lineWidth: 1.6, lineCap: .round))
                .frame(width: 11, height: 11)
            } else {
                Path { p in
                    p.move(to: CGPoint(x: 2, y: 6.5))
                    p.addLine(to: CGPoint(x: 5.2, y: 10))
                    p.addLine(to: CGPoint(x: 11, y: 3.5))
                }
                .stroke(style: StrokeStyle(lineWidth: 1.7, lineCap: .round, lineJoin: .round))
                .frame(width: 13, height: 13)
            }
        }
    }
}
