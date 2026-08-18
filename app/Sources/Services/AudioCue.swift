// 录音提示音 — 移植自原版 audioCue.ts（Web Audio API 即时合成）：
// 上升小三度双音（A5 880Hz → C#6 1108.73Hz），sine 波形，5ms attack + 指数
// release，听感是轻快的「叮咚」。不打包任何音频文件：预渲染 PCM buffer 播放。
// 任何失败静默降级，绝不抛错影响录音主流程（原版设计原则）。

import AVFoundation

final class AudioCue {
    /// 开始录音提示音（capsule:state 进入 recording 时触发；audio_cue_on_record 默认开，
    /// P1.4 设置页接入 prefs 后按配置开关）。
    static func playRecordStart() {
        // 连按热键时先停上一轮，避免叠音（原版 scheduleCueVoices 开头 stopVoices）。
        stop()
        guard ensureCue() else { return }
        player!.scheduleBuffer(buffer!)
        player!.play()
    }

    /// 停止当前提示音（离开 recording 时调用）。
    static func stop() {
        // 不 detach 常驻 player：attach/detach 会让 AudioToolbox 的
        // ListenerMap 每次插入事件监听且从不释放（leaks 实测每 session +2 条）；
        // 常驻节点 attach 一次，永不 detach。
        player?.stop()
    }

    /// 单个正弦音的合成参数（原版 CueTone，值不变）。
    private struct Tone {
        let freq: Double
        let startMs: Double
        let durationMs: Double
        let peakGain: Double
    }

    private static let tones: [Tone] = [
        Tone(freq: 880, startMs: 0, durationMs: 130, peakGain: 0.16),
        Tone(freq: 1108.73, startMs: 95, durationMs: 170, peakGain: 0.18),
    ]

    private static let sampleRate = 44_100.0
    /// 全局共享引擎：makePlayer 里创建的 engine 是局部变量，函数返回即释放，
    /// play() 时 node 找不到引擎 → AVAE_CheckNodeHasEngine 抛异常崩溃。
    /// 引擎必须常驻进程（原版 Web Audio 的 AudioContext 同样是页级单例）。
    private static let engine = AVAudioEngine()
    /// 常驻 player + 预渲染 buffer：首次 ensureCue 时 attach/connect 一次。
    /// 每次 session 只 scheduleBuffer + play（原版每次 new AudioBufferSourceNode
    /// 是 Web Audio 语义，AVAudioPlayerNode 可复用，且复用避免了重复 attach）。
    private static var player: AVAudioPlayerNode?
    private static var buffer: AVAudioPCMBuffer?

    /// 首次调用时预渲染整段提示音到 PCM buffer（44.1kHz 单声道 Float32），
    /// 尾部留 20ms 静音；attach + connect + start 引擎只做一次。
    private static func ensureCue() -> Bool {
        if player != nil { return true }
        let totalMs = tones.reduce(0.0) { max($0, $1.startMs + $1.durationMs) } + 20
        let frameCount = AVAudioFrameCount(sampleRate * totalMs / 1000)
        guard let format = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1),
              let buf = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frameCount)
        else { return false }
        buf.frameLength = frameCount
        guard let samples = buf.floatChannelData?[0] else { return false }

        // 逐采样叠加两个音：attack 5ms 线性（原版指数 ramp 用 0.0001 起步，听感近似），
        // release 指数衰减到尾部（指数不能到 0，压到 1e-4 后直切静音）。
        for tone in tones {
            let start = Int(tone.startMs / 1000 * sampleRate)
            let duration = Int(tone.durationMs / 1000 * sampleRate)
            let attack = Int(0.005 * sampleRate)
            for i in 0..<duration {
                let t = Double(i) / sampleRate
                let env: Double
                if i < attack {
                    env = Double(i) / Double(attack) // 5ms 线性 attack
                } else {
                    let rel = Double(i - attack) / Double(duration - attack)
                    env = pow(0.0001, rel * 4) // 指数 release（4 倍速压到 -80dB）
                }
                let sample = sin(2 * .pi * tone.freq * t) * tone.peakGain * env
                samples[start + i] += Float(sample)
            }
        }

        let p = AVAudioPlayerNode()
        engine.attach(p)
        engine.connect(p, to: engine.mainMixerNode, format: format)
        if !engine.isRunning {
            do {
                try engine.start() // 首次启动引擎；此后常驻，播放即时触发
            } catch {
                return false // 静默降级：无音频会话时放弃
            }
        }
        player = p
        buffer = buf
        return true
    }
}
