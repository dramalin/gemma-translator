# 設計文件：終端機版前端（TUI）

- 日期：2026-10-09
- 狀態：已確認
- 分支：`develop`
- 相關文件：`docs/superpowers/specs/2026-10-09-rust-frontend-design.md`（Rust 前端總體設計）

## 1. 目標與範圍

### 目標
新增一個終端機版的前端 `translator-tui`，在桌機終端機或透過 SSH 使用。功能跟 Slint 版 `translator` 相同，兩者共用同一份狀態邏輯。

### 使用情境
- 在桌機終端機裡操作翻譯，不開圖形視窗。
- 透過 SSH 連到另一台機器（例如 Pi）操作。錄放音用的是**執行程式那台機器**的麥克風和喇叭。

### 成功標準
- 在 SSH 連線的終端機裡能完整翻譯一輪：錄音、辨識、翻譯、播放。
- 中日韓文正確顯示。
- 阿拉伯文內容可以讀；從右到左的顯示取決於終端機（第 5.5 節）。
- `--mock`、`inference_backend = "http"`、`inference_backend = "native"` 三種模式都能用。
- 跟 Slint 版共用 `~/.config/gemma-translator/config.toml`。

### 不在範圍
- 滑鼠操作。
- TUI 自行處理阿拉伯文的雙向排版。
- 只給 TUI 用的設定項。
- 「按住錄音」：終端機普遍收不到放開按鍵的事件，所以 TUI 一律用切換式（第 5.2 節）。

## 2. 現況

`translator-ui`（執行檔 `translator`）目前包含：

| 檔案 | 內容 | 跟 Slint 的關係 |
| --- | --- | --- |
| `src/app.rs` | `AppState`：錄放音（含沒有麥克風時錄靜音）、JobRunner、音量執行緒、controller、設定游標、存檔、http / native / mock services 的選擇 | 只有 `bind_models`、`render`、`set_if_changed` 用到 Slint |
| `src/view.rs` | 把 controller 狀態轉成 lane、狀態列、設定頁各列、音量長條 | 只有 `theme_color` 和 Slint 的 struct 型別用到 Slint |
| `src/mock.rs` | `--mock` 用的假服務 | 無 |
| `src/main.rs` | 命令列解析、`load_config`（含 `.bad` 備份）、Slint 視窗與 30 ms 計時器 | 視窗與計時器 |

`translator-core::keys` 只提供「按住錄音」的對應：按下時開始錄音（`command_for_key_press`），放開時停止（`command_for_key_release`）。

## 3. 架構

```
translator-core   新增切換式錄音的按鍵對應
translator-io     不變
translator-app    新 crate：從 translator-ui 抽出的共用狀態層（不依賴 Slint 或 ratatui）
translator-ui     Slint 執行檔 translator，只剩畫面轉換、按鍵轉換和計時器
translator-tui    新 crate：執行檔 translator-tui（ratatui + crossterm）
```

依賴方向：`translator-ui`、`translator-tui` → `translator-app` → `translator-io`、`translator-core`。

`translator-core` 的規則不變：不依賴 `cpal`、`ureq`、`slint`、`*-sys`，也不依賴 `ratatui` 或 `crossterm`。

## 4. `translator-app`（共用狀態層）

### 4.1 內容
- **從 `translator-ui/src/app.rs` 搬過來：**
  - `AppState`、錄音來源（麥克風或靜音）。
  - services 的選擇（http、native、mock）。
  - 存檔，以及 `pending_capture` 的處理。
- **從 `translator-ui/src/view.rs` 搬過來，改成純資料：**
  - lane、狀態列、設定頁各列、音量長條的計算（`update_levels`）。
- **從 `translator-ui/src/mock.rs` 搬過來：** mock services。
- **從 `translator-ui/src/main.rs` 搬過來：** 命令列解析（`--mock`、`--config <path>`）和 `load_config`（含 `.bad`、`.bad.1` 等備份）。兩個執行檔的參數完全相同。

### 4.2 對外介面

```rust
pub enum RecordStyle { Hold, Toggle }
pub enum KeyAction { Pressed, Released }

impl AppState {
    pub fn new(config: Config, config_path: Option<PathBuf>, is_mock: bool, record_style: RecordStyle) -> Self;
    /// 回傳 true 表示這個鍵被處理了。
    pub fn handle_key(&mut self, key: Key, action: KeyAction) -> bool;
    pub fn handle_focus_lost(&mut self);
    /// 每一幀呼叫：處理 job 事件與音量讀數，推進音量長條。
    pub fn tick(&mut self);
    pub fn view(&self) -> AppView;
}

/// 一幀的畫面內容，純資料，兩個 UI 各自畫出來。
pub struct AppView {
    pub theme_rgb: (u8, u8, u8),
    pub lanes: [LaneView; 2],
    pub show_active_marker: bool,          // landscape 模式才標示作用中的 lane
    pub source_label: String,
    pub transcription: String,
    pub target_label: String,
    pub translation: String,
    pub is_translation_right_to_left: bool,
    pub status: String,
    pub audio_warning: Option<String>,    // 沒有麥克風或喇叭時的說明，TUI 會顯示
    pub levels: Vec<f32>,
    pub is_settings_open: bool,
    pub settings_rows: Vec<SettingsRowView>,
}
```

- **`RecordStyle::Hold`（Slint）：** 行為跟現在完全相同：按下開始錄音，放開停止。
- **`RecordStyle::Toggle`（TUI）：** 錄音鍵的按下改用 `command_for_key_toggle`（第 4.3 節），`Released` 一律忽略。
- 設定頁開著時，按鍵交給設定游標處理，跟現在相同。

### 4.3 `translator-core` 新增的按鍵對應

```rust
pub fn command_for_key_toggle(mode: KeyboardMode, key: Key, active_lane: Lane, phase: Phase) -> Option<Command>;
```

- 錄音鍵在兩種模式的定義跟現有相同：landscape 模式是作用中 lane 的 `Z`；vertical 模式是 lane 1 的 `Z`、lane 2 的 `X`。
  - 如果該 lane 正在錄音（`phase == Recording(lane)`），送出 `StopRecording(lane)`。
  - 否則送出 `StartRecording(lane)`。如果另一個 lane 正在錄音，controller 本來就會忽略這個指令。
- 其他按鍵的結果跟 `command_for_key_press` 相同。

### 4.4 給測試用的建構方式
新增一個不開實體音訊裝置的建構函式：錄音來源固定為靜音，播放用 `DiscardingSink`。這樣 `translator-app` 和 `translator-tui` 的流程測試不需要音訊裝置。

## 5. `translator-tui`

### 5.1 畫面
使用 ratatui，版面跟著終端機大小調整。最小尺寸是 40×12，比這個小時只顯示「視窗太小」，放大後恢復。

```
┌ PERSON 1 ────────────┐┌ PERSON 2 ────────────┐
│ ▶ ARABIC         ◀ ▶ ││   ENGLISH      ● REC │
└──────────────────────┘└──────────────────────┘
 ▁▂▃▅▇█▇▅▃▂▁▁▂▃▅▇
┌ English (Source) ──────────────────────────────┐
│ 逐字稿（自動換行）                              │
├ Chinese (Translation) ─────────────────────────┤
│ 譯文（自動換行）                                │
└────────────────────────────────────────────────┘
 TRANSLATING · Z 錄音  Space 換 lane  ←/→ 語言  Esc 設定  Ctrl-C 離開
```

- **lane：** 作用中的 lane 有 `▶` 標記（只有 landscape 模式）。錄音中的 lane 以反白顯示，並標示 `● REC`。
- **音量長條：** 用 `▁▂▃▄▅▆▇█` 畫出，數量照設定的 `visualizer_bars`；終端機太窄時只顯示最右邊、最新的那幾條。
- **狀態列：**
  - 有錯誤時顯示錯誤訊息，否則顯示目前階段，後面附上依鍵盤模式產生的按鍵提示。
  - 錄音中改提示「再按一次 Z 結束」（vertical 模式則是 Z 或 X）。
  - 閒置時另外提示「點一下錄音鍵，不要按住」。
- **設定頁：** Esc 開啟，以置中的浮動視窗顯示，項目跟 Slint 版相同，用方向鍵操作。
- **顏色：** 主題色當強調色，用在外框、標記和 `● REC`。`COLORTERM` 是 `truecolor` 或 `24bit` 時輸出 RGB；否則換成最接近的具名色或 256 色，免得在不支援 24-bit 色彩的終端機上，錄音中 lane 的字變成黑底黑字。
- **音訊裝置警告：** 沒有麥克風或喇叭時，狀態列一直附上警告（例如 `no microphone: recording silence`）。TUI 的 stderr 被導到 log 檔，沒有這個警告，使用者透過 SSH 只會一直得到「沒有偵測到語音」，卻不知道原因。

### 5.2 按鍵
crossterm 的事件轉成 `translator_core::keys::Key`：

| 終端機按鍵 | `Key` |
| --- | --- |
| 空白鍵 | `Space` |
| `z` / `Z` | `Z` |
| `x` / `X` | `X` |
| `-` / `_` | `Minus` |
| `+` / `=` | `Plus` |
| ← → ↑ ↓ | 對應的方向鍵 |
| Esc | `Escape` |

Ctrl-C 離開。不另外設 `q` 鍵，避免誤觸。帶 Ctrl 或 Alt 的組合鍵一律不當作翻譯器的按鍵，例如 Ctrl-Z 不會開始錄音。TUI 不支援 Ctrl-Z 暫停（suspend）。

**自動重複的過濾：** 終端機收不到放開按鍵的事件。按住 Z 時，作業系統會一直重送「按下」，切換式錄音就會開了又關。處理方式：

- **只過濾錄音鍵（Z、X）：** 同一個錄音鍵上一次按下後 1 秒內再出現的按下，一律當作自動重複而忽略。每一次按下都會更新時間戳，所以整串重複事件都會被濾掉。
- **其他按鍵不過濾：** 快速連按方向鍵切換語言照常有效；按住方向鍵會連續切換。
- **不使用 kitty 鍵盤協定：** 這個協定只有在「所有按鍵都送 escape code」的模式下，才會標明一般字元鍵的重複事件，太複雜。所有終端機都用同一套方式。

過濾器以 `Instant` 為參數，方便測試注入時間。

### 5.3 終端機處理
- 啟動時進入 raw mode 和 alternate screen。不使用 `ratatui::init()`：它的 panic hook 在每條執行緒都會還原終端機，而 job 執行緒的 panic 會被 JobRunner 接住，TUI 其實還在執行。
- 正常結束、收到 SIGTERM 或 SIGHUP 時，都結束主迴圈並還原終端機。
- **panic：** 只有主執行緒的 panic 會還原終端機和 stderr，再印出錯誤和 log 檔路徑。其他執行緒的 panic 照常寫進 log 檔。
- **環境不適合時：** 以下情況直接報錯並結束，exit code 2：
  - stdout 不是 TTY，例如被導到管線；
  - `TERM` 沒有設定或是 `dumb`；
  - 無法進入 raw mode。

### 5.4 stderr 改寫到 log 檔
liblitert-lm、libmoonshine 和程式本身的 `eprintln!` 都直接寫 fd 2，會把畫面弄亂。處理方式：

- 啟動 services 之前，用 `libc::dup` 保存原本的 fd 2，再用 `libc::dup2` 把 fd 2 導到 log 檔。
- log 檔位置是 `$XDG_STATE_HOME/gemma-translator/translator-tui.log`；沒有 `XDG_STATE_HOME` 時用 `~/.local/state/gemma-translator/translator-tui.log`。用附加模式開啟，需要時建立目錄。
- 結束時（包括 panic）先還原 fd 2，再印出 log 檔路徑。
- log 檔開不了時，stderr 維持原樣，啟動前先印出警告，程式照常運作。

這需要新增 `libc` 相依套件，只用在 `translator-tui`。

### 5.5 文字
- 中日韓文的全形寬度由 ratatui 處理（`unicode-width`），換行也會以顯示寬度計算。
- 阿拉伯文照原本的邏輯順序輸出，從右到左的顯示交給終端機：例如 Konsole 或 mlterm 會處理，多數終端機則會顯示成左到右。TUI 不自行重排，以免在本身支援雙向文字的終端機上被反轉兩次。`is_translation_right_to_left` 只用來把該段文字靠右對齊。

### 5.6 主迴圈
每 30 ms 輪詢一次 crossterm 事件，接著執行 `tick()`，再用 `view()` 取得畫面模型並重畫。ratatui 只輸出有變動的部分，所以透過 SSH 時流量也很小。終端機大小改變時重畫。

## 6. `translator-ui` 的改動
- 改成依賴 `translator-app`，使用 `RecordStyle::Hold`。
- 只剩以下工作：
  - 把 `AppView` 轉成 Slint 的 struct 和 `VecModel`（保留「內容有變才更新」）。
  - 把 Slint 的按鍵文字轉成 `Key`。
  - 計時器和焦點事件。
- 使用者看到的行為完全不變。

## 7. 錯誤處理

| 狀況 | 行為 |
| --- | --- |
| 服務沒啟動、找不到函式庫或模型 | 狀態列顯示 `Error: …`，TUI 照常運作（共用邏輯，跟 Slint 版相同） |
| 沒有麥克風或喇叭（例如透過 SSH 的遠端機器沒有音訊裝置，或裝置被另一個程式占用） | 改錄靜音、丟棄要播放的音訊，job 照常完成（共用邏輯）；TUI 的狀態列一直顯示警告 |
| log 檔開不了 | 第 5.4 節 |
| 不是 TTY、`TERM` 沒設定或是 `dumb`、無法進入 raw mode | 報錯並結束，exit code 2 |
| SIGTERM、SIGHUP（例如 `kill`，或 SSH 斷線） | 結束主迴圈，還原終端機 |
| 終端機不支援 24-bit 色彩 | 改用具名色或 256 色 |
| 終端機小於 40×12 | 顯示「視窗太小」，放大後恢復 |
| panic | 先還原終端機和 stderr，再印出錯誤 |

## 8. 測試

`make test` 不需要終端機，也不需要音訊裝置。

| 層 | 測什麼 | 怎麼測 |
| --- | --- | --- |
| `translator-core` | `command_for_key_toggle`：兩種鍵盤模式、正在錄音的 lane 再按一次會停止、錄音中按另一個 lane 的錄音鍵、非錄音鍵的結果跟 `command_for_key_press` 相同 | 單元測試 |
| `translator-app` | 從 translator-ui 搬過來的測試，包括畫面模型、`.bad` 備份、命令列解析；加上用 `Toggle` 和 mock services 跑完整流程：按一下開始、再按一下結束、job 跑完、狀態回到 READY；`Toggle` 模式下 `Released` 被忽略 | 單元測試，用第 4.4 節的建構方式 |
| `translator-tui` | 畫面：用 `TestBackend` 畫出 80×24 和 40×12 兩種尺寸，比對 lane 名稱、狀態列、設定頁；小於 40×12 時顯示提示；中日韓文寬度正確 | 單元測試 |
| `translator-tui` | 按鍵轉換（Ctrl、Alt 組合鍵不算）和自動重複過濾（錄音鍵 1 秒視窗，其他按鍵不過濾） | 單元測試，用注入的 `Instant` |
| `translator-tui` | SIGTERM 會讓主迴圈結束、24-bit 色彩的判斷與退回、`TERM` 的檢查 | 單元測試 |
| `translator-tui` | log 檔路徑的決定（`XDG_STATE_HOME` 和 `HOME` 兩種情況） | 單元測試 |
| `translator-ui` | 搬家後剩下的 Slint 轉換測試，例如主題色和特殊鍵的對應 | 原有測試 |
| 手動 | 透過 SSH 執行 `translator-tui --mock` 和 native 模式各翻譯一輪；確認畫面、中日韓文、按住 Z 不會一直切換、Ctrl-C 之後終端機正常、log 檔有 C 函式庫的輸出 | 交給使用者 |

## 9. 建置與文件
- workspace 新增 `translator-app` 和 `translator-tui` 兩個成員。
- Makefile 新增 `run-tui`：執行 `cargo run -p translator-tui -- $(ARGS)`，mock 模式用 `make run-tui ARGS=--mock`。`make test` 和 `make clippy` 自動涵蓋新的 crate。
- `translator-tui` 不連結 Slint，在沒有圖形環境的機器上也能建置。
- README 和 CLAUDE.md 加上 TUI 的說明：用途、按鍵、log 檔位置，以及阿拉伯文顯示的限制。
