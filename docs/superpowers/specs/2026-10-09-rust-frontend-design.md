# 設計文件：以 Rust 取代 Chromium/React 前端與 Python 後端

- 日期：2026-10-09
- 狀態：草稿 v3（已納入 Fable 審查意見），待確認
- 分支：`develop`
- 參考專案：`~/prog/usbview`（Slint UI、可攜邏輯與硬體分離、mock 模式）

## 1. 目標、範圍與分階段

### 最終目標
執行時只有**一個 Rust 執行檔**：UI、錄放音、語音辨識、翻譯、語音合成都在同一個程序內，執行期不需要 Python、Chromium、Node.js 與 HTTP。

### 分兩個階段，各自可單獨上線

| 階段 | 內容 | 執行期程序 | 主要收益 |
| --- | --- | --- | --- |
| **A：換 UI** | Rust + Slint 取代 Chromium/React；錄放音改在 Rust；推論仍透過 HTTP 呼叫 Python 後端與 `litert-lm serve` | Rust UI、`server.py`、`litert-lm serve` | 記憶體、開機時間、錄放音延遲 |
| **B：換推論層** | Rust 透過 FFI 直接呼叫 `libmoonshine.so`、`liblitert-lm.so`；模型準備改用 Rust 工具 | 只有 Rust 執行檔 | 再省下 Python 記憶體；單一程序；原生串流 |

理由：FFI 有多項待驗證事項（第 11.1 節），階段 A 不受其影響，可先取得大部分好處。兩階段共用同一組推論 trait（第 5.1 節），階段 B 只是替換 trait 的實作。

### 不在範圍
- 觸控操作；更換模型；支援 6 種以外的語言（ar、en、es、ja、zh、ko）。

## 2. 現況

```
Chromium (kiosk) ── React UI ── 錄音 (Web Audio, 前端重新取樣到 16 kHz)
      │  base64 Float32 JSON
      ▼
backend/server.py :3000 ── /api/stt (Moonshine transcribe_without_streaming)
      │                 ── /api/tts (moonshine-voice → WAV)
      │                 ── /proxy → litert-lm :9379
      │                 ── /api/volume (wpctl → pactl → amixer)
      ▼
litert-lm serve :9379 (gemma4-e2b)
```

| 問題 | 影響 |
| --- | --- |
| Chromium 只為了畫 480x320 畫面 | 數百 MB 記憶體，與 Gemma 和語音模型競爭 8 GB；開機慢 |
| 3 個程序另加 Chromium，透過 HTTP 溝通 | 啟動順序、連接埠衝突、單一程序掛掉的處理變複雜 |
| 錄音以 base64 Float32 傳送 | 傳輸量約為原始 16-bit 音訊的 2.7 倍 |
| 每段語音各用一個 HTTP 請求播放 | 段與段之間有停頓 |
| 要求模型回傳嚴格 JSON | 必須等整段完成才能播放；小模型偶爾產生錯誤格式 |

## 3. 必須保留的現行行為

以下行為由程式碼確認，新版本必須重現（「變更」欄標示刻意改變的部分）：

| 行為 | 來源 | 變更 |
| --- | --- | --- |
| 兩種鍵盤模式與按鍵對應（landscape：Space / Z / ← →；vertical：Z / X / ← → / − +） | README、`TranslatorApp.jsx` | 無 |
| 兩個 lane 不可選同一語言，旋轉時跳過另一 lane 的語言 | `TranslatorApp.jsx:127-145` | 無 |
| 錄音中不可切換語言；landscape 錄音中不可切換作用中 lane | `TranslatorApp.jsx` | 無 |
| 開始錄音時停止正在播放的語音 | `TranslatorApp.jsx:154` 附近 | 擴充：同時取消進行中的翻譯（第 6.1 節） |
| 錄音完全沒有樣本時放棄；辨識結果為空時顯示 `(No speech detected)` | `useAudioRecorder.js:118`、`TranslatorApp.jsx:216` | **新增**：錄音短於 300 ms 視為誤觸，直接回到 Idle |
| 提示音：`language`（切換語言）、`ping`、`speaker`（切換作用中 lane / 播放） | `utils/audio-blip.js` | 無 |
| 錄音時的音量長條動畫，長條數可設定 | `components/Visualizer.jsx` | 無 |
| 設定：鍵盤模式、主題色（6 色）、語音輸出開關、長條數、系統音量 | `App.jsx:28-36`、`SettingsOverlay.jsx` | 移除 endpoint / proxy / API key / model（不再需要） |
| 系統音量：wpctl → pactl → amixer 依序嘗試 | `server.py:283-345` | 階段 A 起移到 Rust |
| 開機時預先載入英文的語音辨識與語音合成模型 | `server.py:495-500` | **變更**：改為預先載入兩個 lane 目前的語言 |
| 語音辨識與語音合成模型各最多保留 2 個語言（LRU） | `server.py` `MAX_MODELS = 2` | 無 |
| 播放切段：以空白切分，每段上限約 180 字 | `api.js` `splitTextIntoSpeechChunks` | **變更**：改為依句子標點切分（第 6.3 節）；中日文原本整段不切，新版會切 |
| 設定頁以滑鼠點 ⚙ 開啟 | `App.jsx:73` | **新增**：指定鍵盤按鍵開啟／關閉設定頁（建議 `Esc`），設定頁內以方向鍵操作 |

### 3.1 各語言的模型與聲音

| UI 語言 | 語音辨識模型（`download.py` 取第一個） | 語音合成語言 tag | 聲音 |
| --- | --- | --- | --- |
| en | medium-streaming-en | `en-us` | 預設 |
| ar | base-ar | `ar-msa` | 預設 |
| es | base-es | `es-es` | 預設 |
| ja | base-ja | `ja-jp` | 預設 |
| zh | base-zh | `zh-hans`（moonshine 正規化為 `zh_hans`） | `kokoro_zf_xiaoxiao` |
| ko | tiny-ko（下載名稱為 base-ko） | `ko-kr` | 預設 |

此表定義在 `translator-core`，作為唯一來源；`server.py` 的 `TTS_LANG_MAP` / `TTS_VOICE_MAP` 在階段 B 移除。

## 4. 架構

### 4.1 階段 A

```
translator (Rust)
├── Slint UI (linuxkms，鍵盤經 libinput)
├── 流程控制 (translator-core 狀態機)
├── 錄音 / 播放 / 重新取樣 (cpal)
├── 音量控制 (wpctl → pactl → amixer)
└── HTTP 用戶端
     ├──▶ server.py :3000   POST /v2/stt、POST /v2/tts
     └──▶ litert-lm :9379   /v1/chat/completions
```

### 4.2 階段 B

```
translator (Rust，單一程序)
├── Slint UI、流程控制、錄放音、音量控制（同階段 A）
├── 語音執行緒 ── moonshine-sys ──▶ libmoonshine.so (+ libonnxruntime)
└── 模型執行緒 ── litert-lm-sys ──▶ liblitert-lm.so (+ libvulkan)
```

## 5. Crate 切分（Cargo workspace）

| Crate | 內容 | 階段 | 相依原生庫／OS？ |
| --- | --- | --- | --- |
| `translator-core` | 狀態機、語言表（3.1 節）、按鍵對應、切句、回覆清理、設定資料結構、推論 trait | A | 否 |
| `translator-io` | cpal 錄放音、重新取樣、音量控制、trait 的 HTTP 實作（A）與 FFI 實作（B） | A、B | 是 |
| `translator-ui` | Slint 介面、主程式、`--mock` 模式 | A | 是 |
| `moonshine-sys` | `libmoonshine.so` 的 `extern "C"` 宣告與安全包裝 | B | 是 |
| `litert-lm-sys` | `liblitert-lm.so` 的 `extern "C"` 宣告與安全包裝 | B | 是 |
| `translator-models`（bin） | 部署期下載並準備所有模型檔（第 7.2 節） | B | 是 |

規則：`translator-core` 不得依賴 `*-sys`、`cpal`、`slint`、HTTP 套件。

### 5.1 推論 trait（定義在 `translator-core`）

```rust
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub sample_rate: u32, // 每次合成可能不同，播放前由 translator-io 重新取樣
}

pub trait SpeechToText {
    /// samples 必須是 16 kHz 單聲道。
    fn transcribe(&self, samples: &[f32], language: LanguageCode) -> Result<String, PipelineError>;
}

pub trait Translator {
    /// 每產生一個完整句子就呼叫一次 on_sentence。cancel 被設為 true 時應盡快結束。
    fn translate_streaming(
        &self,
        text: &str,
        source: LanguageCode,
        target: LanguageCode,
        cancel: &AtomicBool,
        on_sentence: &mut dyn FnMut(String),
    ) -> Result<(), PipelineError>;
}

pub trait TextToSpeech {
    fn synthesize(&self, text: &str, language: LanguageCode) -> Result<AudioClip, PipelineError>;
}
```

mock 實作（固定文字、正弦波音訊）讓 UI 在沒有模型的桌機上即可完整操作。

## 6. 流程

### 6.1 狀態機

```
Idle ──按下錄音鍵──▶ Recording ──放開──▶ Transcribing ──▶ Translating ──▶ Speaking ──▶ Idle
  ▲                     │ <300 ms             │ 空結果 → 顯示 (No speech detected)
  └─────────────────────┴─────────────────────┴── 任何錯誤：顯示錯誤訊息後回到 Idle
```

- **中斷**：Transcribing / Translating / Speaking 期間按下任一 lane 的錄音鍵：停止播放、清空播放佇列、設定 `cancel` 旗標（階段 B 另呼叫 `litert_lm_conversation_cancel_process`），然後進入新的 Recording。被中斷的流程丟棄後續結果。
- 語音輸出關閉（`enable_tts = false`）時，Translating 完成後直接回到 Idle，只顯示文字。

### 6.2 執行緒

| 執行緒 | 工作 |
| --- | --- |
| UI 主執行緒 | Slint 事件迴圈；只收發 channel 訊息，不做阻塞工作 |
| 音訊（cpal callback） | 錄音寫入 ring buffer；播放從佇列讀取 |
| 流程執行緒 | 依序執行辨識、翻譯、合成；階段 B 中再細分為語音執行緒與模型執行緒 |

翻譯與播放以管線方式重疊：每切出一句就送去合成，合成好的音訊依序進入播放佇列。

### 6.3 翻譯提示、切句與回覆清理

- 系統提示：「只輸出翻譯結果，不加任何說明」，不再要求 JSON。
- `translator-core` 清理回覆：去除前後綴說明、引號、markdown 符號；Gemma 4 若輸出思考內容（thinking channel），只保留最終回答，不得念出。
- 切句：在 `。！？.!?` 與 `؟` 等結尾標點處切開；超過 180 字仍無標點時，於空白處切開，沒有空白（中日文）則於字元邊界切開。
- 每次翻譯使用**新的對話**，避免上下文在多次翻譯之間累積（現行 HTTP 服務每個請求本來就獨立）。

### 6.4 音訊格式

- 錄音：cpal 以裝置原生取樣率與聲道數擷取，轉成單聲道後重新取樣到 16 kHz。
- 播放：`AudioClip` 帶自己的取樣率，播放前重新取樣到輸出裝置支援的取樣率（Pi 的輸出常只支援 48 kHz）。
- 重新取樣使用 `rubato` crate。

## 7. 階段 A 細節

### 7.1 Python 後端新增 `/v2` 端點（舊端點保留到階段 A 上線）

**`POST /v2/stt?language=<code>`**
- Request body：`application/octet-stream`，16 kHz 單聲道 16-bit little-endian PCM。
- Response：`200 application/json`，`{"text": "..."}`。
- 錯誤：`400` / `500`，body 為 `{"error": "..."}`。

**`POST /v2/tts`**
- Request：`application/json`，`{"text": "...", "language": "<code>"}`（wire JSON 使用 camelCase）。
- Response：`200 application/octet-stream`，16-bit little-endian PCM；取樣率放在 `X-Sample-Rate` header。

### 7.2 翻譯
- 以 `POST http://localhost:9379/v1/chat/completions`，`"stream": true` 透過 SSE 讀取。
- 若 `litert-lm serve` 不支援串流，退回非串流呼叫，完成後再切句。

### 7.3 上線
- systemd 服務改為啟動 litert-lm → `server.py` → `translator`；`deploy-pi.sh` 移除 npm、LXDE 自動啟動與 Chromium kiosk。
- 移除 `frontend/` 與舊 `/api/*`、`/proxy` 端點。

## 8. 階段 B 細節：FFI

### 8.1 依據

拆開 `moonshine-voice` 0.0.65 與 `litert-lm-api` 0.13.1 的 wheel 確認：推論全部在 `libmoonshine.so` 與 `liblitert-lm.so` 內，Python 端以 `ctypes` 呼叫。階段 B 用到的 C 函式：

| 用途 | 函式 | 備註 |
| --- | --- | --- |
| 語音辨識 | `moonshine_load_transcriber_from_files`、`moonshine_transcribe_without_streaming`、`moonshine_free_transcriber` | 結果為 `TranscriptC` / `TranscriptLineC` 結構（`moonshine_api.py:69-104`），需手寫對應的 `#[repr(C)]` |
| 語音合成 | `moonshine_get_tts_voices`、`moonshine_create_tts_synthesizer_from_files`、`moonshine_text_to_speech`、`moonshine_free_tts_synthesizer` | 檔案清單傳 NULL，以 `moonshine_option_t`（`g2p_root`、`voice`）讓 C 端解析；音訊緩衝以 libc `free()` 釋放；取樣率由輸出參數回傳 |
| 模型檔清單 | `moonshine_get_tts_dependencies` | 回傳 JSON 陣列；供 `translator-models` 使用 |
| 錯誤訊息 | `moonshine_error_to_string` | |
| 模型推論 | `litert_lm_engine_settings_create`、`litert_lm_engine_create`、`litert_lm_conversation_config_create`、`litert_lm_conversation_config_set_system_message`、`litert_lm_conversation_create`、`litert_lm_conversation_send_message_stream`、`litert_lm_conversation_cancel_process`、對應的 `*_delete` | 見 8.2 |

G2P（文字轉發音）由語音合成內部處理，不需要自行建立 G2P handle。

### 8.2 litert-lm 串流的實際格式（`litert_lm/conversation.py:55, 146-200`）

- 送出的訊息是 JSON 字串（`{"role":"user","content":[{"type":"text","text":...}]}`），不是純文字。
- callback 簽名：`void (*)(void* user_data, const char* chunk, bool is_final, const char* error)`。
- `chunk` 是 JSON 訊息，需解析出 `content[].text`；解析失敗時才當純文字。
- `error` 為 `"CANCELLED"` 或 `"Max number of tokens reached"` 時視為正常結束，其他值才是錯誤。
- callback 由函式庫內部的 C++ 執行緒呼叫：Rust closure 必須是 `Send`，且存活到 `is_final == true`；callback 內以 `catch_unwind` 防止 panic 跨越 FFI 邊界。

### 8.3 FFI 包裝原則

- wheel 內沒有 C 標頭檔：簽名以 Python `ctypes` 宣告為準手寫；能取得上游標頭檔時改用 `bindgen`。
- 每個 handle 包成擁有所有權的型別，`Drop` 時呼叫對應的釋放函式。
- 執行緒安全性未知：每個引擎只由單一執行緒擁有，其他執行緒以 channel 傳請求。
- 簽名寫錯只會在執行時出錯：每個用到的函式都至少有一個整合測試呼叫到它。

### 8.4 原生函式庫的部署

| 函式庫 | 相依 | 處理 |
| --- | --- | --- |
| `libmoonshine.so` | `libonnxruntime-13ab8084.so.1`，RPATH 為 `$ORIGIN/../moonshine_voice.libs` | 部署時用 `patchelf --set-rpath '$ORIGIN'` 改寫後與 onnxruntime 放在同一目錄 `lib/` |
| `liblitert-lm.so` | `libvulkan.so.1`（不含 onnxruntime，不會與 moonshine 衝突） | Pi 上安裝 `libvulkan1` |

- `.so` 由 `scripts/fetch-native-libs.sh` 從固定版本的 aarch64 wheel 抽出並執行 `patchelf`，不提交進 git。
- `translator` 執行檔以 `rpath = $ORIGIN/lib` 連結。
- 升級版本時必須重新核對所有 FFI 簽名。

### 8.5 模型準備（取代 Python 工具）

現行模型準備依賴 Python：Gemma 用 `litert-lm import`，Moonshine 由 `download.py` 在執行時下載，語音合成需要的檔案清單由 C 函式庫回傳。階段 B 改由 `translator-models` 工具（Rust，載入 `libmoonshine.so`）在部署時處理：

1. Gemma：直接從 Hugging Face `litert-community/gemma-4-E2B-it-litert-lm` 下載 `gemma-4-E2B-it.litertlm`，路徑傳給 `litert_lm_engine_settings_create(path, "cpu", NULL, NULL)`。
2. 語音辨識：依 3.1 節的模型，按 `download.py:168-232` 的規則下載各元件（英文 medium-streaming 另有 `*_with_attention.ort`）。
3. 語音合成：對每個語言呼叫 `moonshine_get_tts_dependencies` 取得檔案清單後下載。
4. 全部放在 `model_dir` 下；`translator` 執行時不連網。

## 9. 設定檔

TOML 檔 `~/.config/gemma-translator/config.toml`（key 使用 snake_case）：

```toml
keyboard_mode = "landscape"   # landscape | vertical
theme_color = "#ffa500"       # 6 個預設色之一
enable_tts = true
visualizer_bars = 16
lane1_language = "en"
lane2_language = "zh"
# 階段 A
speech_service_url = "http://localhost:3000"
llm_endpoint = "http://localhost:9379/v1"
# 階段 B
model_dir = "/var/lib/gemma-translator/models"
# 選填：音訊裝置，未設定則用系統預設
# audio_input_device = ""
# audio_output_device = ""
```

系統音量不寫入設定檔，直接讀寫系統。

## 10. UI

- Slint，`backend-linuxkms` + `renderer-femtovg`；桌機開發用 `backend-winit`，以 Cargo feature 切換。
- 畫面配置、配色、四角括號、音量長條、主題色，以現行 `style.css` 與 `components/` 為準重做。
- 字型隨程式附帶：Roboto Mono、Noto Sans CJK、Noto Sans Arabic。
- 提示音：以程式產生，對應 `audio-blip.js` 的三種音效。

## 11. 遷移步驟

### 階段 A
1. **Slint 可行性實驗**：在 Pi 5 上以 linuxkms 顯示中日韓文、阿拉伯文（含從右到左）與鍵盤輸入。
2. **workspace 與 `translator-core`**：狀態機（含中斷）、語言表、按鍵對應、切句、回覆清理，附單元測試。
3. **`server.py` 新增 `/v2/stt`、`/v2/tts`**，舊端點不動。
4. **`translator-io`**：cpal 錄放音、重新取樣、音量控制、HTTP 實作。
5. **`translator-ui` mock 模式**：桌機完成全部畫面、設定頁與按鍵操作。
6. **Pi 實測與驗收**（第 12 節），通過後切換部署並移除 `frontend/`。

### 階段 B
7. **FFI 可行性實驗**：在 Pi 5 上以最小 Rust 程式驗證 11.1 節所有項目。
8. **`translator-models`**：下載與準備全部模型；在乾淨的 Pi 上驗證，取代 `download_model.sh` 與 Python 下載邏輯。
9. **`moonshine-sys`、`litert-lm-sys`** 與 `translator-io` 的 FFI 實作。
10. **Pi 實測與驗收**，包含長時間執行測試。
11. **切換部署並清理**：移除 `backend/`、`setup.sh`、`start.sh`、`download_model.sh`、venv；更新 README 與 CLAUDE.md。必須在步驟 8 完成後才能移除 Python 工具。

### 11.1 階段 B 實驗必須驗證的項目

| 項目 | 驗證方式 |
| --- | --- |
| aarch64 wheel 是否提供 | `pip download --platform manylinux_2_34_aarch64` |
| glibc 版本（moonshine 要求 2.34，litert-lm 要求 2.27；Bookworm 為 2.36） | 在 Pi 上實際載入 |
| `TranscriptC` 的記憶體由誰釋放（找不到 `moonshine_free_transcript`） | 連續辨識 1000 次觀察 RSS |
| 兩個函式庫同時載入是否衝突 | 同一程序內各執行一次 |
| 串流 callback 的執行緒與格式 | 確認 8.2 節描述 |
| 取消翻譯 | 串流中呼叫 `litert_lm_conversation_cancel_process` |

## 12. 驗收標準

在 Pi 5 8GB 上，以現行版本為基準：

| 指標 | 量測方式 | 目標（實驗後依基準值訂定確切數字） |
| --- | --- | --- |
| 總記憶體 | 所有相關程序 RSS 總和（`smem`） | 階段 A 減少 ≥ 300 MB；階段 B 再減少 Python 程序的用量 |
| 開機到可用 | 開機到可按鍵錄音 | 階段 A 短於基準 |
| 放開按鍵到第一個語音 | 程式內時間戳，固定 5 段測試語句 × 6 語言 | 短於基準 |
| 辨識結果一致 | 同一組錄音檔，新舊版本辨識文字比對 | 完全一致 |
| 顯示 | 6 種語言各顯示一段文字 | 中日韓文、阿拉伯文正確顯示，阿拉伯文從右到左 |
| 功能一致 | 第 3 節每一列逐項測試 | 全部通過 |
| 長時間穩定 | 自動連續翻譯 8 小時 | 無崩潰，RSS 無持續成長 |

## 13. 風險

| 風險 | 處理 |
| --- | --- |
| Slint linuxkms 在 Pi 5 + 目前螢幕不可用，或阿拉伯文排版不足 | 階段 A 步驟 1 先驗證；不足時退回 winit + 輕量 Wayland compositor（cage） |
| FFI 簽名或記憶體所有權錯誤 | 11.1 節實驗；每個函式都有整合測試；長時間測試 |
| moonshine / litert-lm 0.x 版本的 C 介面變動 | 鎖定版本；升級時重新核對 |
| 不改用 JSON 輸出後，小模型多加說明文字或輸出思考內容 | 回覆清理加上測試案例；必要時調整提示，或使用 `litert_lm_conversation_config_set_enable_constrained_decoding` |
| 階段 B 實驗失敗 | 停在階段 A；仍保有大部分收益 |
