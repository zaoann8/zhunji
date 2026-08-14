#![cfg_attr(target_os = "linux", allow(dead_code, unused_variables))]
//! Shared value types crossing the IPC boundary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum PolishMode {
    Raw,
    #[default]
    Light,
    Structured,
    Formal,
}

/// 历史记录的产生来源。旧版 `history.json` 未写入该字段时，按既有听写记录处理。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HistorySource {
    #[default]
    Voice,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ChineseScriptPreference {
    #[default]
    Auto,
    Simplified,
    Traditional,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum OutputLanguagePreference {
    #[default]
    Auto,
    ZhCn,
    ZhTw,
    En,
    Ja,
    Ko,
}

/// 模拟粘贴时实际按下的快捷键。macOS 走 AX 直写 / Cmd+V，本枚举只在
/// Windows / Linux 的 simulate_paste 路径生效。详见 issue #360：kitty 等
/// Linux 终端只接受 Ctrl+Shift+V，硬编码 Ctrl+V 会被吞掉，听写文本只剩
/// 在剪贴板里。默认 `CtrlV` 与历史行为一致；用户在 Settings 里改成
/// `CtrlShiftV`（kitty/alacritty/wezterm/gnome-terminal/foot/...）或
/// `ShiftInsert`（xterm/urxvt）后，simulate_paste 用对应组合。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum PasteShortcut {
    #[default]
    CtrlV,
    CtrlShiftV,
    ShiftInsert,
}

/// Windows 听写文本插入策略。默认 TSF 输入法；SendInput 逐字模拟；Paste 走剪贴板 + 模拟粘贴键。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum WindowsInsertionMode {
    #[default]
    Tsf,
    SendInput,
    Paste,
}

/// Windows SendInput 路径的换行模拟方式。仅 `WindowsInsertionMode::SendInput` 生效。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum WindowsSendInputNewlineMode {
    #[default]
    Enter,
    ShiftEnter,
    CrLf,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InsertStatus {
    Inserted,
    PasteSent,
    CopiedFallback,
    Failed,
}

/// 概览页年度活动热力图的单日计数（date = 本地日期 YYYY-MM-DD）。
#[allow(dead_code)] // P2 概览页活动热力图
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDay {
    pub date: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationSession {
    pub id: String,
    pub created_at: String, // ISO-8601
    /// 本条历史的入口来源。缺失时默认为 `voice`，以兼容既有 history.json。
    #[serde(default)]
    pub source: HistorySource,
    pub raw_transcript: String,
    pub final_text: String,
    pub mode: PolishMode,
    /// 本次 dictation 使用的风格包。旧历史没有此字段时为 None；对话感知 polish
    /// 只复用同一风格包的历史，避免切换风格包后旧上下文污染新提示词。
    #[serde(default)]
    pub style_pack_id: Option<String>,
    /// 本次是否走翻译路径。决定对话感知上下文怎么复用这条历史：下一轮也是翻译时喂
    /// `final_text`（译文）保持一致；下一轮是普通润色时改喂 `polish_source`（润色后的源文）
    /// 以剔除译文、避免外语污染。
    #[serde(default)]
    pub translation_active: bool,
    /// 翻译会话润色后的**源语言**文本（译文前的润色中间产物）。普通会话、解析失败或旧
    /// 历史为 None。仅用于对话感知上下文：普通润色轮复用翻译历史时喂这一段而非译文。
    #[serde(default)]
    pub polish_source: Option<String>,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub insert_status: InsertStatus,
    pub error_code: Option<String>,
    pub duration_ms: Option<u64>,
    pub dictionary_entry_count: Option<u32>,
    /// 当 `prefs.record_audio_for_debug` 开启时，本次会话的原始麦克风音频被写到
    /// `recordings/<id>.wav`。前端凭这个字段决定是否在 History 渲染播放按钮。
    /// `None` / `Some(false)` 都按"无录音"处理；旧 JSON 不带这字段也兼容。
    #[serde(default)]
    pub has_audio_recording: Option<bool>,
    /// 本次转写用的 ASR provider id（如 "volcengine" / "local-qwen3"）。历史详情页
    /// 展示用，方便做模型能力对比。旧历史无此字段时 None，前端隐藏对应行。
    #[serde(default)]
    pub asr_provider: Option<String>,
    /// 本次转写用的 ASR 模型 id。provider 无模型概念（volcengine / apple-speech）时 None。
    #[serde(default)]
    pub asr_model: Option<String>,
    /// 本次润色用的 LLM provider id。Raw 直通（未调用 LLM）时 None。
    #[serde(default)]
    pub llm_provider: Option<String>,
    /// 本次润色用的 LLM 模型 id。Raw 直通时 None。
    #[serde(default)]
    pub llm_model: Option<String>,
    /// 松键后「等待转写结果」的实测耗时（毫秒）。流式 ASR 大部分识别在录音期间已完成，
    /// 这里量的是用户感知的收尾延迟；批式 ASR 则是完整转写耗时。
    #[serde(default)]
    pub asr_ms: Option<u64>,
    /// LLM 润色/翻译调用的实测耗时（毫秒）。未调用 LLM 时 None。
    #[serde(default)]
    pub polish_ms: Option<u64>,
}

fn default_true() -> bool {
    true
}

fn resolve_windows_insertion_mode(
    mode: WindowsInsertionMode,
    legacy_sendinput_only: bool,
) -> WindowsInsertionMode {
    if mode != WindowsInsertionMode::Tsf {
        mode
    } else if legacy_sendinput_only {
        WindowsInsertionMode::SendInput
    } else {
        WindowsInsertionMode::Tsf
    }
}

fn resolve_windows_sendinput_insertion_only_legacy(
    mode: WindowsInsertionMode,
    legacy_sendinput_only: bool,
) -> bool {
    resolve_windows_insertion_mode(mode, legacy_sendinput_only) == WindowsInsertionMode::SendInput
}

#[derive(Debug, Clone, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UserPreferences {
    pub hotkey: HotkeyBinding,
    pub dictation_hotkey: ShortcutBinding,
    pub default_mode: PolishMode,
    pub enabled_modes: Vec<PolishMode>,
    pub launch_at_login: bool,
    pub show_capsule: bool,
    /// 录音胶囊样式：'siri' = 流光 Siri 光效版（默认）；'classic' = Openless 经典药丸版。
    /// 由 capsule:state 事件的 capsuleStyle 字段下发到胶囊 webview，下次录音即生效。
    #[serde(default)]
    pub capsule_style: CapsuleStyle,
    /// 录音期间临时静音系统输出，停止/取消/出错后恢复原静音状态。
    #[serde(default)]
    pub mute_during_recording: bool,
    /// 按下录音热键进入 recording 状态时，播放一段即时合成的提示音，提醒「已开始录音」。
    /// 默认开启；可在「录音与输入」设置里关闭。提示音由 capsule 窗口用 Web Audio API 合成，
    /// 不依赖 show_capsule —— 胶囊隐藏时仍会响。
    #[serde(default = "default_true")]
    pub audio_cue_on_record: bool,
    /// 录音输入设备名称。空字符串 = 使用系统默认麦克风。
    #[serde(default)]
    pub microphone_device_name: String,
    pub active_asr_provider: String, // "volcengine" | "apple-speech" | ...
    pub active_llm_provider: String, // "ark" | "openai" | ...
    /// LLM 思考模式开关。默认 false 以保持既有「尽量关闭思考」行为；
    /// Gemini 走原生 thinkingConfig，OpenAI-compatible 路径仅按 provider/channel
    /// 下发官方渠道级字段；OpenAI 官方渠道会跳过普通 chat 模型不支持的字段。详见 issue #402。
    #[serde(default)]
    pub llm_thinking_enabled: bool,
    /// 是否使用系统代理（issue #869）。默认 true 跟随系统代理，与历史行为一致；
    /// 关闭后所有 reqwest 请求直连（国内服务通常延迟更低），GitHub 登录、更新等
    /// 境外服务可能连不上。实时语音流（WebSocket）与 Less Computer 子进程不受此开关影响。
    #[serde(default = "default_true")]
    pub use_system_proxy: bool,
    /// Windows/Linux 粘贴成功后是否恢复用户原剪贴板。默认 true 跟历史行为一致；
    /// 关掉就把听写文本留在剪贴板，让 simulate_paste 实际没生效时用户能 Ctrl+V 找回。
    /// macOS 走 AX 直写，不受这个开关影响。详见 issue #111。
    pub restore_clipboard_after_paste: bool,
    /// Windows / Linux 的模拟粘贴键。macOS 走 AX 直写不受影响。详见 issue #360：
    /// kitty 等 Linux 终端不接受 Ctrl+V，只能配 Ctrl+Shift+V。默认 CtrlV 与历史
    /// 行为一致，不破坏既有用户。
    #[serde(default)]
    pub paste_shortcut: PasteShortcut,
    /// Windows: 是否允许 TSF 失败后继续使用分批 Unicode SendInput / 剪贴板兜底。
    /// Unicode SendInput 失败时才复制到剪贴板，避免文本丢失。
    /// 默认开启以保持可用性；关闭后可验证文本是否真正由 TSF 上屏。
    #[serde(default = "default_true")]
    pub allow_non_tsf_insertion_fallback: bool,
    /// Windows 听写插入策略：TSF / SendInput / 剪贴板粘贴。
    #[serde(default)]
    pub windows_insertion_mode: WindowsInsertionMode,
    /// Windows SendInput 路径的换行模拟方式。
    #[serde(default, rename = "windowsSendInputNewlineMode")]
    pub windows_sendinput_newline_mode: WindowsSendInputNewlineMode,
    /// 旧版 wire 兼容：`true` 等价于 `windows_insertion_mode = SendInput`。
    #[serde(
        default,
        rename = "windowsSendInputInsertionOnly",
        alias = "windowsSendinputInsertionOnly"
    )]
    pub windows_sendinput_insertion_only: bool,
    /// Windows：SendInput 模式下是否在系统键盘列表（Win+Space）中显示 OpenLess TSF 输入法。
    /// 默认 true 保持现有行为；关闭后用户级禁用语言配置文件，无需管理员权限。
    #[serde(default = "default_true", rename = "windowsShowOpenlessInKeyboardList")]
    pub windows_show_openless_in_keyboard_list: bool,
    /// 用户的工作语言（多选，原生名）。会作为前提注入 LLM polish/translate 的 system prompt 头部，
    /// 让模型知道该用户在哪些语言间工作。详见 issue #4。
    #[serde(default = "default_working_languages")]
    pub working_languages: Vec<String>,
    /// 翻译输出的目标语言（单选，原生名）。空串 = 不启用翻译模式（Shift 组合键无效）。
    /// 由前端从内置语言列表中选择，后端只接收最终的原生名字符串拼进 prompt。详见 issue #4。
    #[serde(default)]
    pub translation_target_language: String,
    /// 中文输出字形偏好（不额外暴露为 UI 开关）：
    /// - Simplified: 中文输出优先简体
    /// - Traditional: 中文输出优先繁体
    /// - Auto: 不额外约束
    ///
    /// 由前端「界面语言」选择同步驱动（简体/繁体），详见 issue #259。
    #[serde(default)]
    pub chinese_script_preference: ChineseScriptPreference,
    /// 最终输出语言偏好（不额外暴露为 UI 开关）：
    /// 由前端「界面语言」选择同步驱动：zh-CN/zh-TW/en/ja/ko，其他为 Auto。
    #[serde(default)]
    pub output_language_preference: OutputLanguagePreference,
    /// 自定义录音组合键。当 `hotkey.trigger == Custom` 时，coordinator 用
    /// `global-hotkey` crate 注册此组合键（支持 Toggle + Hold 模式）。
    /// `None` 且 trigger == Custom 表示用户选了自定义但还没录制。
    #[serde(default)]
    pub custom_combo_hotkey: Option<ComboBinding>,
    #[serde(default = "default_translation_hotkey")]
    pub translation_hotkey: ShortcutBinding,
    /// 「唤起 App」全局快捷键。`None` = 停用；`Some(...)` = 注册。默认 `Some(默认键)`。
    #[serde(default = "default_open_app_hotkey")]
    pub open_app_hotkey: Option<ShortcutBinding>,
    /// 本地 Qwen3-ASR 当前激活的模型 id（"qwen3-asr-0.6b" / "qwen3-asr-1.7b"）。
    /// 仅在 active_asr_provider == "local-qwen3" 时有意义。
    #[serde(default = "default_local_asr_model")]
    pub local_asr_active_model: String,
    /// 本地模型下载源镜像（"huggingface" / "hf-mirror"）。
    #[serde(default = "default_local_asr_mirror")]
    pub local_asr_mirror: String,
    /// 本地 ASR 引擎在内存中的保留时长（秒）。0 = 说完话即释放；
    /// 较大值 = 上次使用后驻留 N 秒再释放；86400 = 一天 ≈ 永不释放。
    /// 默认 300（5 分钟）：兼顾连续听写不重加载、长时间不用释放 1.2GB+ RAM。
    #[serde(default = "default_local_asr_keep_loaded_secs")]
    pub local_asr_keep_loaded_secs: u32,
    /// 本地模型自定义父目录。空字符串 = 使用系统默认 app data 下的 `models/`。
    /// 非空时，实际模型根目录为 `<local_asr_models_base_dir>/OpenLess/models/`，
    /// 让用户选择一个普通磁盘目录即可隔离 OpenLess 模型文件。
    #[serde(default)]
    pub local_asr_models_base_dir: String,
    /// Windows Foundry Local Whisper 当前激活的模型 alias。
    #[serde(default = "default_foundry_local_asr_model")]
    pub foundry_local_asr_model: String,
    /// Windows Foundry Local native runtime 下载源："auto" / "nuget" / "ort-nightly"。
    #[serde(default = "default_foundry_local_runtime_source")]
    pub foundry_local_runtime_source: String,
    /// Windows Foundry Local Whisper 语言 hint。空字符串 = 自动检测。
    #[serde(default)]
    pub foundry_local_asr_language_hint: String,
    /// Windows Foundry Local Whisper 模型在 runtime 中保持加载多久。
    #[serde(default = "default_local_asr_keep_loaded_secs")]
    pub foundry_local_asr_keep_loaded_secs: u32,
    /// Windows sherpa-onnx 本地 ASR 当前激活的模型 alias。
    #[serde(default = "default_sherpa_onnx_model")]
    pub sherpa_onnx_model: String,
    /// Windows sherpa-onnx 语言 hint（BCP-47 / ISO 639-1 小写）。空 = 自动。
    #[serde(default)]
    pub sherpa_onnx_language_hint: String,
    /// Windows sherpa-onnx 模型在 runtime 中保持加载多久（秒），语义与
    /// foundry/qwen3 一致。
    #[serde(default = "default_local_asr_keep_loaded_secs")]
    pub sherpa_onnx_keep_loaded_secs: u32,
    /// 历史记录保留天数。0 = 不按时间清理（仅受 200 条上限）。默认 7 天。
    /// 写入新条目时执行清理，避免后台轮询。
    #[serde(default = "default_history_retention_days")]
    pub history_retention_days: u32,
    /// 对话感知 polish 的上下文窗口（分钟）：把最近 N 分钟的转写 + 已润色文本
    /// 作为多轮上下文喂给 LLM，让代词 / 不完整句子能被正确解析。
    /// 0 = 关闭（每次润色独立单轮，跟历史行为一致）。默认 5 分钟。
    #[serde(default = "default_polish_context_window_minutes")]
    pub polish_context_window_minutes: u32,
    /// 启动时静默运行（不弹主窗口）。开机自启用户用得多——本来想看托盘
    /// 而不是被主窗口打扰。开关一开后所有启动路径都不弹窗（包括手动点击），
    /// 用户改用托盘菜单访问主窗口。默认 false 跟历史行为一致。
    #[serde(default)]
    pub start_minimized: bool,
    /// UI theme: follow OS, force light, or force dark. Frontend applies via data-ol-theme.
    #[serde(default)]
    pub theme_mode: ThemeMode,
    /// 流式输入：润色 SSE 一边到达一边逐字模拟键盘事件输出到当前焦点。开启后用户感知到
    /// 的处理时延显著降低（润色 LLM 第一个 token 即开始落字）。
    ///
    /// 平台原语：
    /// - macOS：CGEvent Unicode FFI；CJK / 日文 IME 会拦截，session 期间临时切到 ABC
    /// - Windows：SendInput Unicode（绕过 TSF）；不需要切输入法
    /// - Linux：通过 fcitx5 插件 commitString 直写或剪贴板回落。
    ///
    /// 限制：
    /// - 不再走剪贴板路径，对 secure input 框（密码框 / 1Password）静默拒绝
    /// - 仅 OpenAI-compatible provider 实装（v1）；Gemini / Codex provider 走原一次性
    ///   插入路径
    ///
    /// 默认 true（自 1.3.2-3 起）—— 流式落字感知延迟低，所有 fallback case 都已经接好，
    /// 让开箱即用就能体验。CJK IME / Codex / Gemini provider 自动回落到一次性路径，
    /// 用户无感。详见上面「限制」段。
    #[serde(default = "default_true")]
    pub streaming_insert: bool,
    /// issue #440 的一次性迁移标记。老版本会把默认 `streamingInsert:false`
    /// 写进 preferences.json，升级后仅看 bool 无法区分「老默认」和「用户手动关」。
    /// 缺少此标记的旧文件统一迁到 true；迁移后用户再关会带着标记保存，后续保留 false。
    #[serde(default)]
    pub streaming_insert_default_migrated: bool,
    /// 流式输入成功后是否把最终润色文本写回剪贴板。一次性路径天然走剪贴板，所以
    /// Cmd+V 可以重复粘贴；流式路径直接合成键盘事件、不动剪贴板，会让用户失去这层
    /// 兜底。开启后流式成功收尾时把 final text 写到系统剪贴板，跟一次性行为对齐。
    /// 默认 true（更接近用户习惯）。
    #[serde(default = "default_true")]
    pub streaming_insert_save_clipboard: bool,
    /// 概览页是否显示「年度活动」热力图卡。默认 true；关闭只隐藏卡片，
    /// 活动计数照常记录（persistence/activity.rs），再打开时全年数据仍在。
    #[serde(default = "default_true")]
    pub show_overview_activity_heatmap: bool,
    /// 主窗口启动 + 后台每 60 分钟自动检查更新。默认 true。
    /// Android 开启后自动检查并下载，校验后打开系统安装器；桌面仅自动检查 + 用户确认安装。
    /// 关闭后仅 Settings 手动「检查更新」按钮可用。
    #[serde(default = "default_true")]
    pub auto_update_check: bool,
    /// 历史记录上限（条数）。`None` = 使用代码内 200 条硬上限；
    /// `Some(n)` 表示用户在 Settings 自定义了上限（5..=200 之间）。
    #[serde(default)]
    pub history_max_entries: Option<u32>,
    /// 是否为每次会话保留原始麦克风音频文件（wav）到 `recordings/` 目录，
    /// 用于排查 ASR 误识别 / 麦克风灵敏度问题。默认 false。开启会占磁盘空间，
    /// 受 `history_retention_days` 同样的清理策略约束。
    #[serde(default)]
    pub record_audio_for_debug: bool,
    /// `recordings/` 里保留的最近 wav 文件数（按 mtime 倒序保留最新的）。
    /// `None` = 跟随 `HISTORY_CAP` (200)；`Some(n)` 时 clamp 到 1..=200。
    /// 调用点：每次开新会话前裁旧。让用户在「文本历史保留 200 条但 wav 只留最近 5 条」
    /// 这种「文本档案多 + 录音不占盘」组合下精确控制。
    #[serde(default)]
    pub audio_recording_max_entries: Option<u32>,
}

impl UserPreferences {
    #[allow(dead_code)] // P2 主题设置：切换时保留样式偏好
    pub(crate) fn preserve_style_preferences_from(&mut self, current: &Self) {
        self.default_mode = current.default_mode;
        self.enabled_modes = current.enabled_modes.clone();
    }
}

fn default_local_asr_model() -> String {
    "qwen3-asr-0.6b".into()
}

fn default_history_retention_days() -> u32 {
    0
}

fn default_polish_context_window_minutes() -> u32 {
    5
}

fn default_local_asr_mirror() -> String {
    "huggingface".into()
}

fn default_local_asr_keep_loaded_secs() -> u32 {
    300
}

fn default_foundry_local_asr_model() -> String {
    "whisper-large-v3-turbo".into()
}

fn default_foundry_local_runtime_source() -> String {
    "auto".into()
}

fn default_sherpa_onnx_model() -> String {
    "qwen3-asr-0.6b-int8".into()
}

fn default_active_asr_provider() -> String {
    "volcengine".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct UserPreferencesWire {
    hotkey: HotkeyBinding,
    dictation_hotkey: Option<ShortcutBinding>,
    default_mode: PolishMode,
    enabled_modes: Vec<PolishMode>,
    launch_at_login: bool,
    show_capsule: bool,
    #[serde(default)]
    capsule_style: CapsuleStyle,
    #[serde(default)]
    mute_during_recording: bool,
    #[serde(default = "default_true")]
    audio_cue_on_record: bool,
    #[serde(default)]
    microphone_device_name: String,
    active_asr_provider: String,
    active_llm_provider: String,
    #[serde(default)]
    llm_thinking_enabled: bool,
    #[serde(default = "default_true")]
    use_system_proxy: bool,
    restore_clipboard_after_paste: bool,
    #[serde(default)]
    paste_shortcut: PasteShortcut,
    allow_non_tsf_insertion_fallback: bool,
    #[serde(default)]
    windows_insertion_mode: WindowsInsertionMode,
    #[serde(
        default,
        rename = "windowsSendInputNewlineMode",
        alias = "windowsSendinputNewlineMode"
    )]
    windows_sendinput_newline_mode: WindowsSendInputNewlineMode,
    #[serde(
        default,
        rename = "windowsSendInputInsertionOnly",
        alias = "windowsSendinputInsertionOnly"
    )]
    windows_sendinput_insertion_only: bool,
    #[serde(default = "default_true", rename = "windowsShowOpenlessInKeyboardList")]
    windows_show_openless_in_keyboard_list: bool,
    working_languages: Vec<String>,
    translation_target_language: String,
    chinese_script_preference: ChineseScriptPreference,
    #[serde(default)]
    output_language_preference: OutputLanguagePreference,
    custom_combo_hotkey: Option<ComboBinding>,
    translation_hotkey: Option<ShortcutBinding>,
    open_app_hotkey: Option<ShortcutBinding>,
    #[serde(default = "default_local_asr_model")]
    local_asr_active_model: String,
    #[serde(default = "default_local_asr_mirror")]
    local_asr_mirror: String,
    #[serde(default = "default_local_asr_keep_loaded_secs")]
    local_asr_keep_loaded_secs: u32,
    #[serde(default)]
    local_asr_models_base_dir: String,
    #[serde(default = "default_foundry_local_asr_model")]
    foundry_local_asr_model: String,
    #[serde(default = "default_foundry_local_runtime_source")]
    foundry_local_runtime_source: String,
    #[serde(default)]
    foundry_local_asr_language_hint: String,
    #[serde(default = "default_local_asr_keep_loaded_secs")]
    foundry_local_asr_keep_loaded_secs: u32,
    #[serde(default = "default_sherpa_onnx_model")]
    sherpa_onnx_model: String,
    #[serde(default)]
    sherpa_onnx_language_hint: String,
    #[serde(default = "default_local_asr_keep_loaded_secs")]
    sherpa_onnx_keep_loaded_secs: u32,
    #[serde(default = "default_history_retention_days")]
    history_retention_days: u32,
    #[serde(default = "default_polish_context_window_minutes")]
    polish_context_window_minutes: u32,
    #[serde(default)]
    start_minimized: bool,
    #[serde(default)]
    theme_mode: ThemeMode,
    #[serde(default = "default_true")]
    streaming_insert: bool,
    #[serde(default)]
    streaming_insert_default_migrated: bool,
    #[serde(default = "default_true")]
    streaming_insert_save_clipboard: bool,
    #[serde(default = "default_true")]
    show_overview_activity_heatmap: bool,
    #[serde(default = "default_true")]
    auto_update_check: bool,
    #[serde(default)]
    history_max_entries: Option<u32>,
    #[serde(default)]
    record_audio_for_debug: bool,
    #[serde(default)]
    audio_recording_max_entries: Option<u32>,
}

impl Default for UserPreferencesWire {
    fn default() -> Self {
        let prefs = UserPreferences::default();
        Self {
            hotkey: prefs.hotkey,
            dictation_hotkey: None,
            default_mode: prefs.default_mode,
            enabled_modes: prefs.enabled_modes,
            launch_at_login: prefs.launch_at_login,
            show_capsule: prefs.show_capsule,
            capsule_style: prefs.capsule_style,
            mute_during_recording: prefs.mute_during_recording,
            audio_cue_on_record: prefs.audio_cue_on_record,
            microphone_device_name: prefs.microphone_device_name,
            active_asr_provider: prefs.active_asr_provider,
            active_llm_provider: prefs.active_llm_provider,
            llm_thinking_enabled: prefs.llm_thinking_enabled,
            use_system_proxy: prefs.use_system_proxy,
            restore_clipboard_after_paste: prefs.restore_clipboard_after_paste,
            paste_shortcut: prefs.paste_shortcut,
            allow_non_tsf_insertion_fallback: prefs.allow_non_tsf_insertion_fallback,
            windows_insertion_mode: prefs.windows_insertion_mode,
            windows_sendinput_newline_mode: prefs.windows_sendinput_newline_mode,
            windows_sendinput_insertion_only: prefs.windows_sendinput_insertion_only,
            windows_show_openless_in_keyboard_list: prefs.windows_show_openless_in_keyboard_list,
            working_languages: prefs.working_languages,
            translation_target_language: prefs.translation_target_language,
            chinese_script_preference: prefs.chinese_script_preference,
            output_language_preference: prefs.output_language_preference,
            custom_combo_hotkey: prefs.custom_combo_hotkey,
            translation_hotkey: None,
            open_app_hotkey: prefs.open_app_hotkey,
            local_asr_active_model: prefs.local_asr_active_model,
            local_asr_mirror: prefs.local_asr_mirror,
            local_asr_keep_loaded_secs: prefs.local_asr_keep_loaded_secs,
            local_asr_models_base_dir: prefs.local_asr_models_base_dir,
            foundry_local_asr_model: prefs.foundry_local_asr_model,
            foundry_local_runtime_source: prefs.foundry_local_runtime_source,
            foundry_local_asr_language_hint: prefs.foundry_local_asr_language_hint,
            foundry_local_asr_keep_loaded_secs: prefs.foundry_local_asr_keep_loaded_secs,
            sherpa_onnx_model: prefs.sherpa_onnx_model,
            sherpa_onnx_language_hint: prefs.sherpa_onnx_language_hint,
            sherpa_onnx_keep_loaded_secs: prefs.sherpa_onnx_keep_loaded_secs,
            history_retention_days: prefs.history_retention_days,
            polish_context_window_minutes: prefs.polish_context_window_minutes,
            start_minimized: prefs.start_minimized,
            theme_mode: prefs.theme_mode,
            streaming_insert: prefs.streaming_insert,
            streaming_insert_default_migrated: prefs.streaming_insert_default_migrated,
            streaming_insert_save_clipboard: prefs.streaming_insert_save_clipboard,
            show_overview_activity_heatmap: prefs.show_overview_activity_heatmap,
            auto_update_check: prefs.auto_update_check,
            history_max_entries: prefs.history_max_entries,
            record_audio_for_debug: prefs.record_audio_for_debug,
            audio_recording_max_entries: prefs.audio_recording_max_entries,
        }
    }
}

impl<'de> Deserialize<'de> for UserPreferences {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = UserPreferencesWire::deserialize(deserializer)?;
        let dictation_hotkey = match wire.dictation_hotkey {
            Some(binding) => binding,
            None => default_dictation_hotkey_from_legacy(&wire.hotkey, &wire.custom_combo_hotkey)
                .map_err(serde::de::Error::custom)?,
        };
        let streaming_insert_default_migrated = wire.streaming_insert_default_migrated;
        let streaming_insert = if streaming_insert_default_migrated {
            wire.streaming_insert
        } else {
            true
        };

        Ok(Self {
            hotkey: wire.hotkey,
            dictation_hotkey,
            default_mode: wire.default_mode,
            enabled_modes: wire.enabled_modes,
            launch_at_login: wire.launch_at_login,
            show_capsule: wire.show_capsule,
            capsule_style: wire.capsule_style,
            mute_during_recording: wire.mute_during_recording,
            audio_cue_on_record: wire.audio_cue_on_record,
            microphone_device_name: wire.microphone_device_name,
            active_asr_provider: wire.active_asr_provider,
            active_llm_provider: wire.active_llm_provider,
            llm_thinking_enabled: wire.llm_thinking_enabled,
            use_system_proxy: wire.use_system_proxy,
            restore_clipboard_after_paste: wire.restore_clipboard_after_paste,
            paste_shortcut: wire.paste_shortcut,
            allow_non_tsf_insertion_fallback: wire.allow_non_tsf_insertion_fallback,
            windows_insertion_mode: resolve_windows_insertion_mode(
                wire.windows_insertion_mode,
                wire.windows_sendinput_insertion_only,
            ),
            windows_sendinput_newline_mode: wire.windows_sendinput_newline_mode,
            windows_sendinput_insertion_only: resolve_windows_sendinput_insertion_only_legacy(
                wire.windows_insertion_mode,
                wire.windows_sendinput_insertion_only,
            ),
            windows_show_openless_in_keyboard_list: wire.windows_show_openless_in_keyboard_list,
            working_languages: wire.working_languages,
            translation_target_language: wire.translation_target_language,
            chinese_script_preference: wire.chinese_script_preference,
            output_language_preference: wire.output_language_preference,
            custom_combo_hotkey: wire.custom_combo_hotkey,
            translation_hotkey: wire
                .translation_hotkey
                .unwrap_or_else(default_translation_hotkey),
            open_app_hotkey: wire.open_app_hotkey,
            local_asr_active_model: wire.local_asr_active_model,
            local_asr_mirror: wire.local_asr_mirror,
            local_asr_keep_loaded_secs: wire.local_asr_keep_loaded_secs,
            local_asr_models_base_dir: wire.local_asr_models_base_dir,
            foundry_local_asr_model: wire.foundry_local_asr_model,
            foundry_local_runtime_source: wire.foundry_local_runtime_source,
            foundry_local_asr_language_hint: wire.foundry_local_asr_language_hint,
            foundry_local_asr_keep_loaded_secs: wire.foundry_local_asr_keep_loaded_secs,
            sherpa_onnx_model: wire.sherpa_onnx_model,
            sherpa_onnx_language_hint: wire.sherpa_onnx_language_hint,
            sherpa_onnx_keep_loaded_secs: wire.sherpa_onnx_keep_loaded_secs,
            history_retention_days: wire.history_retention_days,
            polish_context_window_minutes: wire.polish_context_window_minutes,
            start_minimized: wire.start_minimized,
            theme_mode: wire.theme_mode,
            streaming_insert,
            streaming_insert_default_migrated: true,
            streaming_insert_save_clipboard: wire.streaming_insert_save_clipboard,
            show_overview_activity_heatmap: wire.show_overview_activity_heatmap,
            auto_update_check: wire.auto_update_check,
            history_max_entries: wire.history_max_entries,
            record_audio_for_debug: wire.record_audio_for_debug,
            audio_recording_max_entries: wire.audio_recording_max_entries,
        })
    }
}

impl UserPreferences {
    /// 逐字段抢救一份无法严格反序列化的 preferences.json。
    ///
    /// 背景：`UserPreferencesWire` 容器级 `#[serde(default)]` 已能容忍「缺字段」
    /// （老文件读新版本）。真正会让整份解析失败、进而静默回落默认值（= 用户所有
    /// 设置一次性丢光）的，是「字段存在但值非法」——例如某次重构改了枚举变体名 /
    /// 字段类型，旧文件里的旧值在新版本里不再合法。这正是用户反馈「每次重装 app
    /// 之后热键等设置就读不到」的根因路径。
    ///
    /// 抢救策略：把 JSON 当作对象，先归一化已知 alias，再逐 key 试解析。因为 Wire 对
    /// 所有字段都有 default，单键对象 `{k: v}` 只有当 `v` 对字段 `k` 的类型非法时才会
    /// 失败——据此精确剔除坏字段，保留其余全部有效设置（热键、模型选择、风格等都能
    /// 活下来），最后再走一次正常反序列化。无法当作对象解析时才彻底回落默认。
    pub(crate) fn salvage_from_json_bytes(bytes: &[u8]) -> Self {
        let Ok(serde_json::Value::Object(mut map)) =
            serde_json::from_slice::<serde_json::Value>(bytes)
        else {
            return Self::default();
        };

        normalize_preference_aliases(&mut map);

        let mut cleaned = serde_json::Map::new();
        for (key, value) in map {
            if preference_field_is_valid(&key, &value) {
                cleaned.insert(key, value);
            } else {
                log::warn!("[prefs] salvage dropping unparseable field: {key}");
            }
        }

        match serde_json::from_value::<Self>(serde_json::Value::Object(cleaned.clone())) {
            Ok(prefs) => prefs,
            Err(err) => {
                if let Some(prefs) = salvage_without_incomplete_legacy_hotkey(cleaned) {
                    return prefs;
                }
                log::warn!(
                    "[prefs] salvage still failed after field filtering: {err}; using defaults"
                );
                Self::default()
            }
        }
    }
}

fn preference_field_is_valid(key: &str, value: &serde_json::Value) -> bool {
    let probe =
        serde_json::Value::Object(std::iter::once((key.to_string(), value.clone())).collect());
    serde_json::from_value::<UserPreferencesWire>(probe).is_ok()
}

fn normalize_preference_aliases(map: &mut serde_json::Map<String, serde_json::Value>) {
    for (canonical, alias) in [
        ("windowsSendInputNewlineMode", "windowsSendinputNewlineMode"),
        (
            "windowsSendInputInsertionOnly",
            "windowsSendinputInsertionOnly",
        ),
    ] {
        let Some(alias_value) = map.remove(alias) else {
            continue;
        };
        let canonical_valid = map
            .get(canonical)
            .map(|value| preference_field_is_valid(canonical, value));
        let alias_valid = preference_field_is_valid(canonical, &alias_value);

        match canonical_valid {
            None => {
                map.insert(canonical.to_string(), alias_value);
            }
            Some(true) => log::warn!(
                "[prefs] salvage dropping duplicate legacy alias {alias}; canonical {canonical} wins"
            ),
            Some(false) if alias_valid => {
                log::warn!(
                    "[prefs] salvage replacing invalid canonical {canonical} with valid legacy alias {alias}"
                );
                map.insert(canonical.to_string(), alias_value);
            }
            Some(false) => {}
        }
    }
}

fn salvage_without_incomplete_legacy_hotkey(
    mut map: serde_json::Map<String, serde_json::Value>,
) -> Option<UserPreferences> {
    let is_custom_legacy_hotkey = map
        .get("hotkey")
        .and_then(|value| value.get("trigger"))
        .and_then(serde_json::Value::as_str)
        == Some("custom");
    if !is_custom_legacy_hotkey {
        return None;
    }

    let has_dictation_hotkey = map
        .get("dictationHotkey")
        .and_then(|value| serde_json::from_value::<Option<ShortcutBinding>>(value.clone()).ok())
        .flatten()
        .is_some();
    let has_custom_combo_hotkey = map
        .get("customComboHotkey")
        .and_then(|value| serde_json::from_value::<Option<ComboBinding>>(value.clone()).ok())
        .flatten()
        .is_some();
    if has_dictation_hotkey || has_custom_combo_hotkey {
        return None;
    }

    map.remove("hotkey");
    serde_json::from_value::<UserPreferences>(serde_json::Value::Object(map)).ok()
}

fn default_translation_hotkey() -> ShortcutBinding {
    ShortcutBinding {
        primary: "Shift".into(),
        modifiers: Vec::new(),
    }
}

fn default_open_app_hotkey() -> Option<ShortcutBinding> {
    Some(ShortcutBinding {
        primary: "O".into(),
        modifiers: default_app_shortcut_modifiers(),
    })
}

fn default_app_shortcut_modifiers() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        vec!["cmd".into(), "shift".into()]
    }
    #[cfg(not(target_os = "macos"))]
    {
        vec!["ctrl".into(), "shift".into()]
    }
}

fn default_dictation_hotkey_from_legacy(
    hotkey: &HotkeyBinding,
    custom_combo_hotkey: &Option<ComboBinding>,
) -> Result<ShortcutBinding, String> {
    if hotkey.trigger == HotkeyTrigger::Custom {
        if let Some(combo) = custom_combo_hotkey {
            return Ok(ShortcutBinding {
                primary: combo.primary.clone(),
                modifiers: combo.modifiers.clone(),
            });
        }
        return Err(
            "hotkey.trigger is custom but dictationHotkey/customComboHotkey is missing".into(),
        );
    }
    Ok(crate::shortcut_binding::binding_from_legacy_trigger(
        hotkey.trigger,
    ))
}

fn default_working_languages() -> Vec<String> {
    vec!["简体中文".into()]
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            hotkey: HotkeyBinding::default(),
            dictation_hotkey: default_dictation_hotkey_from_legacy(
                &HotkeyBinding::default(),
                &None,
            )
            .expect("default legacy hotkey is not custom"),
            default_mode: PolishMode::Structured,
            enabled_modes: vec![
                PolishMode::Raw,
                PolishMode::Light,
                PolishMode::Structured,
                PolishMode::Formal,
            ],
            launch_at_login: false,
            show_capsule: true,
            capsule_style: CapsuleStyle::Siri,
            mute_during_recording: false,
            audio_cue_on_record: true,
            microphone_device_name: String::new(),
            active_asr_provider: default_active_asr_provider(),
            active_llm_provider: "ark".into(),
            llm_thinking_enabled: false,
            use_system_proxy: true,
            restore_clipboard_after_paste: true,
            paste_shortcut: PasteShortcut::default(),
            allow_non_tsf_insertion_fallback: true,
            windows_insertion_mode: WindowsInsertionMode::default(),
            windows_sendinput_newline_mode: WindowsSendInputNewlineMode::default(),
            windows_sendinput_insertion_only: false,
            windows_show_openless_in_keyboard_list: true,
            working_languages: default_working_languages(),
            translation_target_language: String::new(),
            chinese_script_preference: ChineseScriptPreference::Auto,
            output_language_preference: OutputLanguagePreference::Auto,
            custom_combo_hotkey: None,
            translation_hotkey: default_translation_hotkey(),
            open_app_hotkey: default_open_app_hotkey(),
            local_asr_active_model: default_local_asr_model(),
            local_asr_mirror: default_local_asr_mirror(),
            local_asr_keep_loaded_secs: default_local_asr_keep_loaded_secs(),
            local_asr_models_base_dir: String::new(),
            foundry_local_asr_model: default_foundry_local_asr_model(),
            foundry_local_runtime_source: default_foundry_local_runtime_source(),
            foundry_local_asr_language_hint: String::new(),
            foundry_local_asr_keep_loaded_secs: default_local_asr_keep_loaded_secs(),
            sherpa_onnx_model: default_sherpa_onnx_model(),
            sherpa_onnx_language_hint: String::new(),
            sherpa_onnx_keep_loaded_secs: default_local_asr_keep_loaded_secs(),
            history_retention_days: default_history_retention_days(),
            polish_context_window_minutes: default_polish_context_window_minutes(),
            start_minimized: false,
            theme_mode: ThemeMode::default(),
            streaming_insert: true,
            streaming_insert_default_migrated: true,
            streaming_insert_save_clipboard: true,
            show_overview_activity_heatmap: true,
            auto_update_check: true,
            history_max_entries: None,
            record_audio_for_debug: false,
            audio_recording_max_entries: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    pub primary: String,
    pub modifiers: Vec<String>,
}

/// 录音快捷键的自定义组合键绑定：
/// - `primary`：主键（如 `"D"`、`"Space"`、`"F1"`）。
/// - `modifiers`：修饰键集合，元素来自 `{"cmd","ctrl","alt","shift","super"}`。
///
/// 当 `HotkeyBinding.trigger == Custom` 时，coordinator 用 `global-hotkey` crate
/// 注册此组合键，而非 modifier-only 的 CGEventTap / WH_KEYBOARD_LL。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ComboBinding {
    pub primary: String,
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyTrigger {
    RightOption,
    LeftOption,
    RightControl,
    LeftControl,
    RightCommand,
    LeftCommand,
    LeftShift,
    RightShift,
    Fn,
    RightAlt, // Windows synonym for RightOption
    MediaPlayPause,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyMode {
    Toggle,
    Hold,
    DoubleClick,
    /// 自动识别：按下即开录；松手时按「按住时长」决定语义 —— 短按（< AUTO_HOLD_THRESHOLD）
    /// 当作 Toggle（锁存，保持录音，下次按下再停），长按当作 Hold（松手即停）。
    Auto,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyAdapterKind {
    MacEventTap,
    WindowsLowLevel,
    Fcitx5,
    /// Mobile platforms do not expose desktop global hotkey adapters.
    Unavailable,
}

impl HotkeyAdapterKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            HotkeyAdapterKind::MacEventTap => "macOS Event Tap",
            HotkeyAdapterKind::WindowsLowLevel => "Windows 低层键盘 hook",
            HotkeyAdapterKind::Fcitx5 => "fcitx5 输入法插件",
            HotkeyAdapterKind::Unavailable => "不可用",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyKey {
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct HotkeyBinding {
    pub trigger: HotkeyTrigger,
    pub mode: HotkeyMode,
    pub keys: Option<Vec<HotkeyKey>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCapability {
    pub adapter: HotkeyAdapterKind,
    pub available_triggers: Vec<HotkeyTrigger>,
    pub requires_accessibility_permission: bool,
    pub supports_modifier_only_trigger: bool,
    pub supports_side_specific_modifiers: bool,
    pub explicit_fallback_available: bool,
    pub status_hint: Option<String>,
}

impl HotkeyCapability {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                adapter: HotkeyAdapterKind::MacEventTap,
                available_triggers: vec![
                    HotkeyTrigger::RightOption,
                    HotkeyTrigger::LeftOption,
                    HotkeyTrigger::RightControl,
                    HotkeyTrigger::LeftControl,
                    HotkeyTrigger::RightCommand,
                    HotkeyTrigger::LeftCommand,
                    HotkeyTrigger::LeftShift,
                    HotkeyTrigger::RightShift,
                    HotkeyTrigger::Fn,
                    HotkeyTrigger::Custom,
                ],
                requires_accessibility_permission: true,
                supports_modifier_only_trigger: true,
                supports_side_specific_modifiers: true,
                explicit_fallback_available: false,
                status_hint: Some("授权辅助功能后，通常需要完全退出并重新打开 OpenLess。".into()),
            }
        }

        #[cfg(target_os = "windows")]
        {
            return Self {
                adapter: HotkeyAdapterKind::WindowsLowLevel,
                // Windows 没有 Command 键：leftCommand/rightCommand 会被映射到 Win 键，
                // 而单按 Win 会弹出开始菜单，实际无法作为录音热键使用。故不在 Windows
                // 的常用单键预设里提供 Command 选项（issue #784）。
                available_triggers: vec![
                    HotkeyTrigger::RightControl,
                    HotkeyTrigger::RightAlt,
                    HotkeyTrigger::LeftControl,
                    HotkeyTrigger::LeftShift,
                    HotkeyTrigger::RightShift,
                    HotkeyTrigger::MediaPlayPause,
                    HotkeyTrigger::Custom,
                ],
                requires_accessibility_permission: false,
                supports_modifier_only_trigger: true,
                supports_side_specific_modifiers: true,
                explicit_fallback_available: false,
                status_hint: Some(
                    "默认建议使用“右Ctrl + 单击”；若更习惯按住说话，可在录音设置里切回“按住”。若无响应，可在权限页查看 hook 安装状态。"
                        .into(),
                ),
            };
        }

        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            Self {
                adapter: HotkeyAdapterKind::Fcitx5,
                available_triggers: vec![
                    HotkeyTrigger::RightAlt,
                    HotkeyTrigger::RightControl,
                    HotkeyTrigger::LeftControl,
                    HotkeyTrigger::LeftCommand,
                    HotkeyTrigger::LeftShift,
                    HotkeyTrigger::RightShift,
                    HotkeyTrigger::Custom,
                ],
                requires_accessibility_permission: false,
                supports_modifier_only_trigger: true,
                supports_side_specific_modifiers: true,
                explicit_fallback_available: false,
                status_hint: Some(
                    "Linux 使用 fcitx5 插件监听热键和提交文字。鼠标/侧别组合键需 evdev 读取 /dev/input/event*；若无权限请将用户加入 input 组（sudo usermod -aG input $USER）后重新登录。"
                        .into(),
                ),
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyInstallError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for HotkeyInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    pub adapter: HotkeyAdapterKind,
    pub state: HotkeyStatusState,
    pub message: Option<String>,
    pub last_error: Option<HotkeyInstallError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(dead_code)] // P2 概览页「平台能力」卡片
#[serde(rename_all = "camelCase")]
pub struct PlatformCapabilities {
    pub platform: String,
    pub supports_ime_input: bool,
    pub supports_overlay: bool,
    pub supports_desktop_hotkey: bool,
    pub supports_tray: bool,
    pub supports_local_asr: bool,
    pub supports_in_app_dictation: bool,
    pub supports_auto_update: bool,
}

impl PlatformCapabilities {
    #[allow(dead_code)] // P2 概览页
    pub fn current() -> Self {
        Self {
            platform: "desktop".to_string(),
            supports_ime_input: cfg!(target_os = "windows"),
            supports_overlay: true,
            supports_desktop_hotkey: true,
            supports_tray: true,
            supports_local_asr: cfg!(any(target_os = "macos", target_os = "windows")),
            supports_in_app_dictation: false,
            supports_auto_update: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum HotkeyStatusState {
    Starting,
    Installed,
    Failed,
}

impl Default for HotkeyStatus {
    fn default() -> Self {
        Self {
            adapter: HotkeyCapability::current().adapter,
            state: HotkeyStatusState::Starting,
            message: Some("正在安装全局快捷键监听".into()),
            last_error: None,
        }
    }
}

impl Default for HotkeyBinding {
    fn default() -> Self {
        // 注意：keys 必须是 None，不能预填具体 code。
        //
        // 原因：HotkeyBinding 用 `#[serde(default)]` **结构级 default**——反序列化时
        // 整个 struct 先按 Default 填充再让 JSON 字段覆盖。如果这里 keys 预填了
        // Some([...])，那么旧 prefs 里只写 `{"trigger":"rightControl","mode":"toggle"}`
        // （不带 keys 字段）会被反序列化成 `{trigger=RightControl, keys=Some([默认值])}`
        // 即 trigger 跟 keys 完全不一致——effective_codes() 直接信任 keys，导致
        // 实际生效的快捷键跟用户当年选的 trigger 对不上。
        // 现在 keys=None 时 effective_codes() 走 legacy_trigger_code(trigger) 路径，
        // 跟 trigger 自动同步。
        #[cfg(target_os = "windows")]
        {
            Self {
                trigger: HotkeyTrigger::RightControl,
                mode: HotkeyMode::Toggle,
                keys: None,
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            Self {
                trigger: HotkeyTrigger::RightOption,
                mode: HotkeyMode::Toggle,
                keys: None,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CapsuleState {
    Idle,
    Recording,
    Transcribing,
    Polishing,
    Done,
    Cancelled,
    Error,
}

/// 录音胶囊样式。由 UserPreferences.capsule_style 透传到 capsule:state payload，
/// 胶囊 webview 据此选择渲染流光 Siri 光效舞台还是经典药丸。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CapsuleStyle {
    /// 流光 Siri 风格：SiriGL 光效舞台（默认）。
    #[default]
    Siri,
    /// Openless 默认风格：经典毛玻璃药丸（音量条 + 取消/确认按钮）。
    Classic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsulePayload {
    pub state: CapsuleState,
    pub level: f32, // 0..1 RMS
    pub elapsed_ms: u64,
    pub message: Option<String>,
    pub inserted_chars: Option<u32>,
    /// 当前 session 是否处于翻译模式（用户按过 Shift）。前端用它在胶囊顶部
    /// 渲染"正在翻译"标签，让用户立刻知道这次输出会走翻译管线。详见 issue #4。
    pub translation: bool,
    /// 预备态：胶囊已经"乐观显示"出来（按下热键即弹出并播入场动画），但麦克风还没
    /// 真正开始 capture 第一帧 PCM。为 true 时前端渲染"待命"光效（柔和呼吸、不接真实
    /// 电平），并暗示用户先别急着开口；`level_handler` 首次触发（PCM 真的流入）后翻成
    /// false，光条"点亮"进入正式录音态。只对 Recording 状态有意义。详见胶囊出现时序改造。
    #[serde(default)]
    pub warming: bool,
    /// 用户选择的胶囊样式（siri / classic）。随每次状态事件下发，设置里切换后下一次
    /// 录音即生效，胶囊 webview 无需额外请求。
    #[serde(default)]
    pub capsule_style: CapsuleStyle,
}

/// Snapshot of credentials read from vault — only what the UI needs to know
/// (whether keys are set; never the values themselves).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[allow(dead_code)] // P2 设置页凭据状态卡
#[serde(rename_all = "camelCase")]
pub struct CredentialsStatus {
    pub active_asr_provider: String,
    pub active_llm_provider: String,
    pub asr_configured: bool,
    pub llm_configured: bool,
    // 兼容旧前端字段（逐步迁移中）
    pub volcengine_configured: bool,
    pub ark_configured: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salvage_preserves_valid_fields_when_one_value_is_invalid() {
        // 模拟「某次重构改了枚举变体名」后的旧文件：defaultMode 是新版本已不存在的值，
        // 但 dictationHotkey / activeAsrProvider 仍然合法。抢救必须保住合法字段，
        // 只把非法字段回落默认——而不是整份丢光。
        let json = br#"{
            "defaultMode": "totally-removed-mode",
            "dictationHotkey": { "primary": "LeftOption", "modifiers": [] },
            "activeAsrProvider": "bailian-qwen3-realtime"
        }"#;

        // 严格解析必失败（否则这个测试没意义）。
        assert!(serde_json::from_slice::<UserPreferences>(json).is_err());

        let salvaged = UserPreferences::salvage_from_json_bytes(json);
        assert_eq!(salvaged.dictation_hotkey.primary, "LeftOption");
        assert_eq!(salvaged.active_asr_provider, "bailian-qwen3-realtime");
        // 非法字段回落到默认，而不是让整份解析失败。
        assert_eq!(
            salvaged.default_mode,
            UserPreferences::default().default_mode
        );
    }

    #[test]
    fn salvage_normalizes_duplicate_legacy_aliases_without_resetting_other_fields() {
        let json = br#"{
            "windowsSendInputInsertionOnly": false,
            "windowsSendinputInsertionOnly": true,
            "windowsSendInputNewlineMode": "removed-mode",
            "windowsSendinputNewlineMode": "shiftEnter",
            "activeAsrProvider": "preserved-provider"
        }"#;

        assert!(serde_json::from_slice::<UserPreferences>(json).is_err());

        let salvaged = UserPreferences::salvage_from_json_bytes(json);
        assert!(!salvaged.windows_sendinput_insertion_only);
        assert_eq!(
            salvaged.windows_sendinput_newline_mode,
            WindowsSendInputNewlineMode::ShiftEnter
        );
        assert_eq!(salvaged.active_asr_provider, "preserved-provider");
    }

    #[test]
    fn non_tsf_insertion_fallback_defaults_to_enabled() {
        let prefs = UserPreferences::default();

        assert!(prefs.allow_non_tsf_insertion_fallback);
    }

    #[test]
    fn missing_non_tsf_insertion_fallback_pref_defaults_to_enabled() {
        let prefs: UserPreferences = serde_json::from_str("{}").unwrap();

        assert!(prefs.allow_non_tsf_insertion_fallback);
    }

    #[test]
    fn windows_sendinput_insertion_only_defaults_to_disabled() {
        let prefs = UserPreferences::default();
        assert!(!prefs.windows_sendinput_insertion_only);
        assert_eq!(prefs.windows_insertion_mode, WindowsInsertionMode::Tsf);

        let prefs: UserPreferences = serde_json::from_str("{}").unwrap();
        assert!(!prefs.windows_sendinput_insertion_only);
        assert_eq!(prefs.windows_insertion_mode, WindowsInsertionMode::Tsf);
    }

    #[test]
    fn windows_sendinput_insertion_only_deserializes_frontend_wire_key() {
        let prefs: UserPreferences =
            serde_json::from_str(r#"{"windowsSendInputInsertionOnly": true}"#).unwrap();
        assert!(prefs.windows_sendinput_insertion_only);
        assert_eq!(
            prefs.windows_insertion_mode,
            WindowsInsertionMode::SendInput
        );
    }

    #[test]
    fn windows_sendinput_insertion_only_deserializes_legacy_wrong_camel_key() {
        let prefs: UserPreferences =
            serde_json::from_str(r#"{"windowsSendinputInsertionOnly": true}"#).unwrap();
        assert!(prefs.windows_sendinput_insertion_only);
        assert_eq!(
            prefs.windows_insertion_mode,
            WindowsInsertionMode::SendInput
        );
    }

    #[test]
    fn windows_insertion_mode_deserializes_explicit_paste() {
        let prefs: UserPreferences =
            serde_json::from_str(r#"{"windowsInsertionMode":"paste"}"#).unwrap();
        assert_eq!(prefs.windows_insertion_mode, WindowsInsertionMode::Paste);
        assert!(!prefs.windows_sendinput_insertion_only);
    }

    #[test]
    fn windows_sendinput_newline_mode_defaults_to_enter() {
        let prefs: UserPreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(
            prefs.windows_sendinput_newline_mode,
            WindowsSendInputNewlineMode::Enter
        );
    }

    #[test]
    fn windows_sendinput_newline_mode_deserializes_shift_enter() {
        let prefs: UserPreferences =
            serde_json::from_str(r#"{"windowsSendInputNewlineMode":"shiftEnter"}"#).unwrap();
        assert_eq!(
            prefs.windows_sendinput_newline_mode,
            WindowsSendInputNewlineMode::ShiftEnter
        );
    }

    #[test]
    fn windows_sendinput_newline_mode_serializes_frontend_wire_key() {
        let prefs = UserPreferences {
            windows_insertion_mode: WindowsInsertionMode::SendInput,
            windows_sendinput_newline_mode: WindowsSendInputNewlineMode::ShiftEnter,
            ..UserPreferences::default()
        };
        let json = serde_json::to_string(&prefs).unwrap();
        assert!(json.contains(r#""windowsSendInputNewlineMode":"shiftEnter""#));
        assert!(!json.contains("windowsSendinputNewlineMode"));
    }

    #[test]
    fn windows_sendinput_insertion_only_serializes_frontend_wire_key() {
        let enabled = UserPreferences {
            windows_insertion_mode: WindowsInsertionMode::SendInput,
            windows_sendinput_insertion_only: true,
            ..UserPreferences::default()
        };
        let json = serde_json::to_string(&enabled).unwrap();
        assert!(json.contains(r#""windowsSendInputInsertionOnly":true"#));
        assert!(!json.contains("windowsSendinputInsertionOnly"));
    }

    #[test]
    fn windows_sendinput_insertion_only_pref_round_trips_explicit_true() {
        let enabled = UserPreferences {
            windows_insertion_mode: WindowsInsertionMode::SendInput,
            windows_sendinput_insertion_only: true,
            ..UserPreferences::default()
        };
        let json = serde_json::to_string(&enabled).unwrap();
        assert!(json.contains(r#""windowsSendInputInsertionOnly":true"#));
        assert!(json.contains(r#""windowsInsertionMode":"sendInput""#));
        let restored: UserPreferences = serde_json::from_str(&json).unwrap();
        assert!(restored.windows_sendinput_insertion_only);
        assert_eq!(
            restored.windows_insertion_mode,
            WindowsInsertionMode::SendInput
        );
    }

    #[test]
    fn windows_show_openless_in_keyboard_list_defaults_to_enabled() {
        let prefs = UserPreferences::default();
        assert!(prefs.windows_show_openless_in_keyboard_list);

        let prefs: UserPreferences = serde_json::from_str("{}").unwrap();
        assert!(prefs.windows_show_openless_in_keyboard_list);
    }

    #[test]
    fn windows_show_openless_in_keyboard_list_deserializes_frontend_wire_key() {
        let prefs: UserPreferences =
            serde_json::from_str(r#"{"windowsShowOpenlessInKeyboardList": false}"#).unwrap();
        assert!(!prefs.windows_show_openless_in_keyboard_list);
    }

    #[test]
    fn windows_show_openless_in_keyboard_list_serializes_frontend_wire_key() {
        let hidden = UserPreferences {
            windows_show_openless_in_keyboard_list: false,
            ..UserPreferences::default()
        };
        let json = serde_json::to_string(&hidden).unwrap();
        assert!(json.contains(r#""windowsShowOpenlessInKeyboardList":false"#));
    }

    #[test]
    fn missing_audio_cue_on_record_pref_defaults_to_enabled() {
        // 老用户的 preferences.json 没有这个字段 → 应默认开启（按下录音即提示）。
        let prefs: UserPreferences = serde_json::from_str("{}").unwrap();

        assert!(prefs.audio_cue_on_record);
    }

    #[test]
    fn audio_cue_on_record_pref_round_trips_explicit_false() {
        // 用户在设置里关掉后，set_settings → 存盘 → get_settings 必须保住 false，
        // 否则开关一刷新又跳回 true（字段在 Wire 往返时被丢掉的经典症状）。
        let disabled = UserPreferences {
            audio_cue_on_record: false,
            ..Default::default()
        };
        let json = serde_json::to_string(&disabled).unwrap();
        assert!(
            json.contains("\"audioCueOnRecord\":false"),
            "序列化应输出 camelCase 字段，实际: {json}"
        );

        let restored: UserPreferences = serde_json::from_str(&json).unwrap();
        assert!(!restored.audio_cue_on_record);
    }

    #[test]
    fn open_app_hotkey_defaults_to_enabled() {
        // issue #576：默认仍开启（Some 默认键），对老用户零行为变化。
        let prefs = UserPreferences::default();
        assert!(prefs.open_app_hotkey.is_some());
    }

    #[test]
    fn missing_open_app_hotkey_defaults_to_enabled() {
        // 老用户/缺字段：wire 的 struct-default 落到 Some(默认键)，不应被当成停用。
        let prefs: UserPreferences = serde_json::from_str("{}").unwrap();
        assert!(prefs.open_app_hotkey.is_some());
    }

    #[test]
    fn disabled_open_app_hotkey_round_trips_as_null() {
        // issue #576：用户清空（None=停用）后存盘→读回必须仍是 None，
        // 不能像旧逻辑那样被 unwrap_or_else 塌缩回默认键。
        let disabled = UserPreferences {
            open_app_hotkey: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&disabled).unwrap();
        assert!(
            json.contains("\"openAppHotkey\":null"),
            "停用应序列化成 null，实际: {json}"
        );
        let restored: UserPreferences = serde_json::from_str(&json).unwrap();
        assert!(restored.open_app_hotkey.is_none());
    }

    /// issue #360: 默认值必须是 CtrlV，跟历史行为一致；老配置文件没有
    /// pasteShortcut 字段时反序列化也得回到 CtrlV，否则会把现有用户的粘贴
    /// 行为静默改掉。
    #[test]
    fn paste_shortcut_defaults_to_ctrl_v() {
        let prefs = UserPreferences::default();
        assert_eq!(prefs.paste_shortcut, PasteShortcut::CtrlV);

        let from_empty: UserPreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(from_empty.paste_shortcut, PasteShortcut::CtrlV);
    }

    /// issue #440: 老版本会把默认 `streamingInsert:false` 写进 preferences.json。
    /// 缺少迁移标记的旧文件统一迁到 true；带有迁移标记后，用户再手动关掉的 false
    /// 必须保留。
    #[test]
    fn streaming_insert_defaults_to_enabled_for_missing_or_legacy_unmigrated_pref() {
        let prefs = UserPreferences::default();
        assert!(prefs.streaming_insert);
        assert!(prefs.streaming_insert_default_migrated);
        assert!(prefs.streaming_insert_save_clipboard);

        let from_empty: UserPreferences = serde_json::from_str("{}").unwrap();
        assert!(from_empty.streaming_insert);
        assert!(from_empty.streaming_insert_default_migrated);
        assert!(from_empty.streaming_insert_save_clipboard);

        let from_legacy_false: UserPreferences = serde_json::from_str(
            r#"{
                "streamingInsert": false,
                "streamingInsertSaveClipboard": true
            }"#,
        )
        .unwrap();
        assert!(from_legacy_false.streaming_insert);
        assert!(from_legacy_false.streaming_insert_default_migrated);
    }

    #[test]
    fn streaming_insert_preserves_explicit_disabled_value() {
        let prefs: UserPreferences = serde_json::from_str(
            r#"{
                "streamingInsert": false,
                "streamingInsertDefaultMigrated": true,
                "streamingInsertSaveClipboard": false
            }"#,
        )
        .unwrap();

        assert!(!prefs.streaming_insert);
        assert!(prefs.streaming_insert_default_migrated);
        assert!(!prefs.streaming_insert_save_clipboard);
    }

    #[test]
    fn paste_shortcut_round_trips_explicit_values() {
        for (raw, expected) in [
            ("ctrlV", PasteShortcut::CtrlV),
            ("ctrlShiftV", PasteShortcut::CtrlShiftV),
            ("shiftInsert", PasteShortcut::ShiftInsert),
        ] {
            let json = format!(r#"{{ "pasteShortcut": "{raw}" }}"#);
            let prefs: UserPreferences = serde_json::from_str(&json).unwrap();
            assert_eq!(prefs.paste_shortcut, expected, "raw={raw}");
        }
    }

    #[test]
    fn legacy_custom_hotkey_without_custom_binding_is_rejected() {
        let result = serde_json::from_str::<UserPreferences>(
            r#"{
                "hotkey": { "trigger": "custom", "mode": "toggle" }
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn salvage_preserves_valid_fields_when_legacy_custom_hotkey_is_incomplete() {
        let json = br#"{
            "hotkey": { "trigger": "custom", "mode": "toggle", "keys": null },
            "activeAsrProvider": "preserved-provider"
        }"#;

        assert!(serde_json::from_slice::<UserPreferences>(json).is_err());

        let salvaged = UserPreferences::salvage_from_json_bytes(json);
        assert_eq!(salvaged.active_asr_provider, "preserved-provider");
        assert_eq!(salvaged.hotkey, UserPreferences::default().hotkey);
    }

    #[test]
    fn legacy_custom_hotkey_uses_custom_combo_binding() {
        let prefs: UserPreferences = serde_json::from_str(
            r#"{
                "hotkey": { "trigger": "custom", "mode": "toggle" },
                "customComboHotkey": { "primary": "D", "modifiers": ["cmd", "shift"] }
            }"#,
        )
        .unwrap();

        assert_eq!(prefs.dictation_hotkey.primary, "D");
        assert_eq!(prefs.dictation_hotkey.modifiers, vec!["cmd", "shift"]);
    }

    #[test]
    fn custom_hotkey_with_dictation_hotkey_preserves_dictation_binding() {
        let prefs: UserPreferences = serde_json::from_str(
            r#"{
                "hotkey": { "trigger": "custom", "mode": "toggle" },
                "dictationHotkey": { "primary": "Space", "modifiers": ["ctrl"] }
            }"#,
        )
        .unwrap();

        assert_eq!(prefs.dictation_hotkey.primary, "Space");
        assert_eq!(prefs.dictation_hotkey.modifiers, vec!["ctrl"]);
    }

    /// PR #826：新增的模型/耗时字段必须向后兼容——旧 history.json 完全没有这些 key。
    #[test]
    fn dictation_session_deserializes_legacy_json_without_model_fields() {
        let legacy = r#"{
            "id": "abc",
            "createdAt": "2026-07-01T00:00:00Z",
            "rawTranscript": "你好",
            "finalText": "你好。",
            "mode": "light",
            "appBundleId": null,
            "appName": null,
            "insertStatus": "inserted",
            "errorCode": null,
            "durationMs": 1200,
            "dictionaryEntryCount": null
        }"#;
        let session: DictationSession = serde_json::from_str(legacy).expect("legacy json");
        assert_eq!(session.source, HistorySource::Voice);
        assert_eq!(session.asr_provider, None);
        assert_eq!(session.asr_model, None);
        assert_eq!(session.llm_provider, None);
        assert_eq!(session.llm_model, None);
        assert_eq!(session.asr_ms, None);
        assert_eq!(session.polish_ms, None);
    }

    /// 新字段序列化必须是 camelCase（前端 types.ts 镜像按 camelCase 读）。
    #[test]
    fn dictation_session_serializes_model_fields_as_camel_case() {
        let session = DictationSession {
            id: "abc".into(),
            created_at: "2026-07-01T00:00:00Z".into(),
            source: HistorySource::Voice,
            raw_transcript: "你好".into(),
            final_text: "你好。".into(),
            mode: PolishMode::Light,
            style_pack_id: None,
            translation_active: false,
            polish_source: None,
            app_bundle_id: None,
            app_name: None,
            insert_status: InsertStatus::Inserted,
            error_code: None,
            duration_ms: Some(1200),
            dictionary_entry_count: None,
            has_audio_recording: None,
            asr_provider: Some("bailian".into()),
            asr_model: Some("fun-asr-realtime".into()),
            llm_provider: Some("ark".into()),
            llm_model: Some("deepseek-v3-2".into()),
            asr_ms: Some(230),
            polish_ms: Some(1450),
        };
        let json = serde_json::to_value(&session).expect("serialize");
        assert_eq!(json["source"], "voice");
        assert_eq!(json["asrProvider"], "bailian");
        assert_eq!(json["asrModel"], "fun-asr-realtime");
        assert_eq!(json["llmProvider"], "ark");
        assert_eq!(json["llmModel"], "deepseek-v3-2");
        assert_eq!(json["asrMs"], 230);
        assert_eq!(json["polishMs"], 1450);
    }
}
