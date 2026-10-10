# 設計文件：macOS 支援與環境設定腳本

- 日期：2026-10-10
- 狀態：待審
- 分支：`develop`
- 相關文件：`docs/superpowers/specs/2026-10-09-rust-frontend-design.md`、`docs/superpowers/specs/2026-10-09-tui-frontend-design.md`

## 1. 目標與範圍

### 目標
1. 讓整個專案能在 Apple Silicon、macOS 15 以上的 Mac 上當開發機和正式翻譯機使用：Rust 的 Slint 版和 TUI 版（`http` 與 `native` 兩種推論後端），以及 Python 後端（`setup.sh`、`start.sh` 的 HTTP 模式）。
2. 新增 `make setup`：檢查系統工具、自動補齊專案需要下載的檔案，Linux 和 macOS 都適用。

### 使用情境
- 使用者從終端機手動啟動（`make run-native`、`make run-tui`、`./start.sh`）。麥克風權限給終端機 App。
- 使用者有 macOS 15 以上的 Apple Silicon Mac，會照第 8 節的清單手動驗收。

### 成功標準
- 在 Mac 上 `make setup` 之後，`make run-native` 和 `make run-tui` 能用 native 後端完整翻譯一輪（錄音、辨識、翻譯、播放）。
- 在 Mac 上 `./setup.sh && ./download_model.sh && ./start.sh` 能用 HTTP 模式完整翻譯一輪。
- Mac 上音量鍵能調整系統輸出音量，並顯示正確的百分比，上限 100%。
- Linux 上所有現有行為不變；`make test`、`make clippy`、`cargo fmt --check` 全過。
- `make setup` 在一台全新的機器上會列出缺少的系統工具；工具齊全時補齊所有專案檔案；重複執行時不會重新下載已經有的東西。

### 不在範圍
- Intel Mac（moonshine-voice 沒有 x86_64 的 macOS wheel）。
- macOS 15 以前的版本（moonshine-voice 的 macOS wheel 標示 `macosx_15_0_arm64`）。
- `.app` 打包、程式碼簽章、launchd 自動啟動、kiosk 模式。
- macOS CI。
- 自動安裝系統工具（`brew install`、`apt-get install`）：只檢查並列出安裝指令。

## 2. 已查證的事實

| 項目 | 結果 |
|---|---|
| pinned wheel 的 macOS 版本 | `moonshine_voice-0.0.65-py3-none-macosx_15_0_arm64.whl`（sha256 `3e92fa8497bf4fc40b6241e4dc29fd4ac77832f7f3d8addc4fd467a6bf1cf605`）、`litert_lm_api-0.13.1-py3-none-macosx_12_0_arm64.whl`（sha256 `4a1bb68dbf5a60af9ac5fe03331968a7b4e91779e721cb531e6b41e71f2a2d77`） |
| wheel 內的函式庫 | `moonshine_voice/libmoonshine.dylib`、`moonshine_voice/libonnxruntime.1.23.2.dylib`、`litert_lm/liblitert-lm.dylib` |
| 函式庫路徑 | `libmoonshine.dylib` 以 `@rpath/libonnxruntime.1.23.2.dylib` 載入 onnxruntime，且自帶 `LC_RPATH @loader_path`；三個檔案放在同一個資料夾即可，不需要 `install_name_tool` 或 `patchelf`。其他依賴都是系統 framework。 |
| ctypes 簽名 | Linux aarch64 與 macOS arm64 wheel 裡的 24 個 `.py` 檔 CRC 完全相同，所以 `-sys` crate 的函式簽名在 macOS 上一樣適用。 |
| Python 依賴 | `backend/requirements.txt` 對每個含二進位檔的套件都已鎖了 macOS arm64（或 universal2）wheel 的 hash，`pip install --require-hashes` 在 Mac 上可用。 |
| `litert-lm import` | 支援本機檔案：`litert-lm import ./model.litertlm my-model`。 |
| macOS 內建 bash | 3.2，不支援 `start.sh` 用到的 `wait -n`（需要 4.3）。`lsof`、`nc`、`ping`、`unzip`、`curl`、`shasum` 都有；沒有 `sha256sum`、`wpctl`、`pactl`、`amixer`、`patchelf`。 |

## 3. 做法

平台差異用編譯期 `#[cfg(target_os = "macos")]`（Rust）和 `sys.platform == "darwin"` / `uname -s`（Python、shell）分流。不新增 crate，也不新增設定項。

## 4. 元件設計

### 4.1 `scripts/fetch-native-libs.sh`
- 用法改成 `scripts/fetch-native-libs.sh [x86_64|aarch64|arm64]`，預設 `uname -m`。`arm64` 只在 `uname -s` 為 `Darwin` 時有效；在 Darwin 上傳 `x86_64` 會報錯「Intel Mac 不支援」並以 2 結束。
- Darwin arm64 使用第 2 節的兩個 wheel 與 sha256；Linux 兩組不變。
- 檢查 hash：有 `sha256sum` 就用，否則用 `shasum -a 256 -c`。
- 必要工具：`curl`、`unzip`、上面兩者之一；只有 Linux 需要 `patchelf`。缺工具時的提示依平台給 `brew install` 或 `sudo apt-get install`。
- Darwin 解出 `libmoonshine.dylib`、`libonnxruntime.1.23.2.dylib`、`liblitert-lm.dylib` 到 `native/arm64/lib/`，不做 rpath 修改。最後一樣寫 `.complete`。

### 4.2 `moonshine-sys`、`litert-lm-sys`
- `LIBRARY_FILE_NAME` / `LITERT_LM_LIBRARY_FILE_NAME` 依 `target_os` 定義：macOS 為 `libmoonshine.dylib` / `liblitert-lm.dylib`，其他為現有的 `.so`。
- 錯誤訊息裡寫死的 `libmoonshine.so` / `liblitert-lm.so` 改成引用常數。
- `litert-lm-sys/tests/engine.rs`、`translator-io/src/native/speech.rs`、`translator-io/src/native/translator.rs` 裡比對檔名的測試改用常數。
- crate 文件第一行改成「`libmoonshine` / `liblitert-lm`」，不寫副檔名。

### 4.3 `translator-io/src/volume.rs`
- `VolumeControl::current_percent` 和 `adjust` 在 macOS 上改用 `osascript`：
  - 讀取：`osascript -e "output volume of (get volume settings)"`，輸出例如 `75\n`；用新函式 `parse_osascript_volume` 解析成 0–100，非數字或 `missing value` 回 `None`。
  - 調整：先讀目前音量 `p`，算出 `min(p + 5, 100)` 或 `p.saturating_sub(5)`，執行 `osascript -e "set volume output volume N"`，再讀一次回傳。
- Linux 的 `wpctl` → `pactl` → `amixer` 順序不變。
- `SystemCommandRunner` 補 `XDG_RUNTIME_DIR` 的那段只在 Linux 編譯。
- `VolumeControl` 新增私有欄位 `platform: VolumePlatform`（`enum VolumePlatform { Linux, MacOs }`）。`VolumeControl::new` 依 `cfg!(target_os = "macos")` 設定它；同模組的測試直接建構 `VolumeControl { runner, platform: VolumePlatform::MacOs }`。這樣 macOS 分支和 `parse_osascript_volume` 在 Linux 上也會編譯和測試，不會只有在 Mac 上才被編譯到。

### 4.4 `backend/server.py`
- `/api/volume` 在 `sys.platform == "darwin"` 時：
  - `get_vol` 用 `osascript -e "output volume of (get volume settings)"`。
  - `set_vol` 讀目前音量後設成 ±5、夾在 0–100 之間。
- Linux 分支與 `XDG_RUNTIME_DIR` 處理不變，只是在 Darwin 上不設 `XDG_RUNTIME_DIR`。
- 解析與夾值抽成模組層級的小函式，加進 `backend/test_*.py` 的單元測試。

### 4.5 `start.sh`
- `XDG_RUNTIME_DIR` 的 export 只在 Linux 做。
- 開機把音量調到最大：Darwin 用 `osascript -e "set volume output volume 100"`，Linux 不變。
- `wait -n` 改成 bash 3.2 相容的輪詢：每秒用 `kill -0` 檢查 `LITERT_PID`、`API_PID`、`WEB_PID`（有設的才檢查），任一個結束就印出現有訊息並 `exit 1`。

### 4.6 `scripts/setup-env.sh` 與 `make setup`
`make setup` 執行 `scripts/setup-env.sh $(SETUP_ARGS)`；腳本接受 `--yes`（不詢問直接下載模型）和 `--help`。

**步驟 1：檢查系統工具**，收集所有缺少的項目，最後一次列出，有任何必要項缺少就以 1 結束、不進入步驟 2。

| 工具 | 平台 | 檢查方式 | 缺少時的提示 |
|---|---|---|---|
| `cargo` | 全部 | `command -v cargo` | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| `python3` ≥ 3.10 | 全部 | `python3 -c 'import sys; sys.exit(sys.version_info < (3, 10))'` | `brew install python@3.12` / `sudo apt-get install -y python3 python3-venv` |
| `curl`、`unzip` | 全部 | `command -v` | `brew install …` / `sudo apt-get install -y …` |
| `sha256sum` 或 `shasum` | 全部 | 任一存在 | `sudo apt-get install -y coreutils` |
| `patchelf`、`pkg-config` | Linux | `command -v` | `sudo apt-get install -y …` |
| ALSA 開發檔 | Linux | `pkg-config --exists alsa` | `sudo apt-get install -y libasound2-dev` |
| Apple Silicon | macOS | `uname -m` 為 `arm64` | 結束並說明不支援 Intel Mac |
| macOS ≥ 15 | macOS | `sw_vers -productVersion` 主版號 | 結束並說明需要 macOS 15 |
| Xcode Command Line Tools | macOS | `xcode-select -p` | `xcode-select --install` |
| `npm`（選用） | 全部 | `command -v npm` | 只警告：React 網頁介面需要 |

**步驟 2：補齊專案檔案**，每項先檢查是否已就緒，已就緒就跳過：

| 項目 | 已就緒的判斷 | 動作 |
|---|---|---|
| Python venv | `venv/bin/litert-lm` 存在且可執行 | 執行 `./setup.sh` |
| native 函式庫 | `native/<arch>/lib/.complete` 存在 | `make native-libs` |
| 模型 | `translator-models` 的檢查通過（見下） | 詢問後 `make models` |
| litert-lm 匯入的 Gemma | `litert-lm list` 的 ID 欄有 `gemma4-e2b` | `./download_model.sh` |
| `frontend/node_modules` | 資料夾存在 | 有 `npm` 才 `npm --prefix frontend install`，沒有則跳過 |

- 「模型已就緒」：`make models` 本身已會跳過已下載的檔案；腳本以 `models/gemma/gemma-4-E2B-it.litertlm` 與 `models/stt/`、`models/tts/` 是否存在且非空作為快速判斷，不存在才詢問。詢問時說明約 3.8 GB；非互動（stdin 不是 TTY）且沒有 `--yes` 時跳過並在總表標示。
- `download_model.sh` 改成：若 `models/gemma/gemma-4-E2B-it.litertlm` 存在，改用 `litert-lm import <該檔> gemma4-e2b` 匯入本機檔案，不再從 Hugging Face 下載；否則維持原本的下載匯入。

**步驟 3：總表**，每項一行，標示 `ok`（已就緒）、`installed`（這次補齊）、`skipped`（使用者略過或缺選用工具）、`FAILED`（附補救指令）。任何一項 `FAILED` 時以 1 結束；否則提示 `make run-native` / `make run-tui` / `./start.sh`。

失敗處理：步驟 2 的某一項失敗時記為 `FAILED` 並繼續下一項（彼此沒有依賴的項目），最後統一回報；`make models` 依賴 native 函式庫，前者失敗時後者記為 `skipped`。

### 4.7 Makefile
- 新增 `setup` 目標。
- `NATIVE_ARCH` 沿用 `uname -m`（Mac 上為 `arm64`），`native/arm64/lib` 即為 Mac 的路徑；`run-native` 的 symlink 做法不變。
- 檔頭註解改成「`make run-mock` works on any Linux or macOS desktop」。

### 4.8 文件
- README：
  - 新增「macOS (Apple Silicon)」小節：需求（macOS 15、Xcode CLT、Homebrew 選用）、`make setup`、第一次錄音時要允許終端機使用麥克風、不支援 Intel Mac。
  - Setup Instructions 開頭加上 `make setup`。
- CLAUDE.md：Commands 加 `make setup`；Rust UI 段落加一行「macOS 上函式庫是 `.dylib`，靠 wheel 自帶的 `@loader_path` rpath，不需要 patchelf；音量用 `osascript`」。

## 5. 資料流
沒有改變。只有函式庫檔名、音量指令和腳本的平台分支。

## 6. 錯誤處理
- 在 Intel Mac 或 macOS 15 以前的版本執行 `fetch-native-libs.sh` 或 `make setup`：明確訊息，非 0 結束。
- `osascript` 失敗或逾時：同 Linux 指令失敗，`current_percent` 回 `None`，介面顯示沒有音量。
- 在 macOS 上沒拿到麥克風權限：cpal 會收到靜音或開啟失敗，沿用現有的音訊警告；README 說明到「系統設定 → 隱私權與安全性 → 麥克風」打開終端機的權限。

## 7. 測試
- Rust：
  - `parse_osascript_volume` 的單元測試：`75\n`、`0`、`100`、`missing value`、空字串。
  - 用 `FakeRunner` 測 macOS 分支的 `current_percent` 與 `adjust`：上調夾在 100、下調夾在 0、讀取失敗回 `None`。
  - 檔名常數：在 Linux 上斷言為 `.so`。
- Python：音量解析與夾值的單元測試，加進現有的 `backend/test_*.py`。
- Shell：`bash -n` 檢查 `start.sh`、`scripts/fetch-native-libs.sh`、`scripts/setup-env.sh`；在這台 Linux 上實際執行 `make setup`，確認全部 `ok`，且第二次執行沒有任何下載。
- 交叉編譯：`rustup target add aarch64-apple-darwin` 後 `cargo check --target aarch64-apple-darwin -p moonshine-sys -p litert-lm-sys -p translator-core`；含 cpal、Slint 的 crate 若因缺 macOS SDK 無法交叉編譯，就記錄下來，交給第 8 節在 Mac 上驗收。
- 既有檢查：`make test`、`make clippy`、`cargo fmt --all -- --check`。

## 8. 手動驗收（使用者在 Mac 上）
1. 全新 clone，執行 `make setup`：缺工具時列出 `brew` 指令；裝好後再執行，全部補齊。
2. 再執行一次 `make setup`：全部 `ok`，沒有下載。
3. `make test`、`make clippy` 全過。
4. `make run-native`：允許終端機使用麥克風，完整翻譯一輪中文 → 英文。
5. `make run-tui`（`inference_backend = "native"`）：同上。
6. 音量鍵：調整時系統音量跟著變，到 100% 停止。
7. `./start.sh`：在瀏覽器開 http://localhost:5173 完整翻譯一輪；按 Ctrl-C 後三個程序都結束。
8. `make test-ffi`：全部通過。
