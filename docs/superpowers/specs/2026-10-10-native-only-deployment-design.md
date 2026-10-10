# 設計文件：只用 native 推論，切換 Pi 部署並移除 Python 與 React

- 日期：2026-10-10
- 狀態：待審（已依 Fable 審查修訂）
- 分支：`develop`
- 相關文件：`docs/superpowers/specs/2026-10-09-rust-frontend-design.md`（第 8.4、11、12 節）、`docs/superpowers/specs/2026-10-10-macos-support-design.md`

## 1. 目標與範圍

### 目標
完成原設計文件第 11 節剩下的兩件事，並讓 Raspberry Pi 5 與 macOS 走同一條路：
1. 推論只走 native（行程內 FFI），刪除 HTTP 推論路徑、Python 後端、React 前端與相關腳本。
2. Pi 的部署改成：在 x86_64 Linux 桌機上交叉編譯並打包，把 tar 檔複製到 Pi 上執行 `sudo ./install.sh`，開機直接進入 Slint（linuxkms）翻譯畫面。

### 前提（已確認）
- Pi 5 與 Apple Silicon Mac 都已用 native 模式通過驗收（2026-10-10）。
- 使用者不會把翻譯模型放在其他機器上，所以不需要保留 HTTP 翻譯客戶端。
- Pi 版只在 x86_64 Linux 桌機上編譯（`cross`），不在 Pi 上、也不在 Mac 上編。
- 模型一起打包。
- Mac 從終端機手動啟動，不做自動啟動或 `.app`。

### 支援的 Pi 系統
**Raspberry Pi OS Bookworm（64 位元、含桌面版）**。這個版本已內建執行時需要的 Mesa（EGL/GLES、GBM、Vulkan）、PipeWire、字型與 ALSA 工具，所以安裝時不需要網路。Lite 版不支援：缺少的套件需要上網安裝，`install.sh` 會列出指令（見 3.5）。

### 假設（由驗收確認）
先前在 Pi 上驗證的是 linuxkms 的可行性實驗（階段 A 步驟 1）與 native 推論（階段 B 步驟 10）。完整程式以 `--features kms`、由 systemd 在沒有桌面的情況下啟動，是本次第一次執行，由第 6 節的 Pi 驗收確認。

### 成功標準
- 在支援的 Pi 系統上執行 `sudo ./install.sh` 並重開機後，直接進入翻譯畫面，錄音、辨識、翻譯、播放與音量鍵都正常，安裝過程不需要網路。
- 在 Mac 或 Linux 桌機上，`make setup && make run` 能直接用 native 模式翻譯。
- repo 裡不再有 Python 或 Node 的程式碼與腳本。
- `make test`、`make clippy`、`cargo fmt --all -- --check` 全過。

### 不在範圍
- 在 Pi 或 Mac 上編譯 Pi 版。
- Pi 上的網路安裝、遠端部署（`deploy-pi` 經 SSH）。
- Raspberry Pi OS Lite 的離線安裝。
- Mac 的 `.app`、launchd、簽章。
- 移除安裝（uninstall）腳本；README 只說明如何改回桌面開機。
- `translator-tui` 放進 Pi 套件。

## 2. 現況
- `translator-core::config::Config` 有 `inference_backend`（預設 `http`）、`speech_service_url`、`llm_endpoint`、`llm_model`；`Config` 以 `#[serde(default)]` 解析，不拒絕未知的鍵。設定頁沒有顯示這四個欄位。
- `translator-app/src/state.rs`：`AppState::new` 把 `config` 交給 `Controller`，存檔時序列化 `controller.config()`（`state.rs:211, 359-366`）；`assemble` 依 `inference_backend` 選 HTTP 或 `native_services(config)`（`state.rs:371-399`），後者用 `config.model_dir` 建 `ModelPaths`。
- `translator-io` 的 `speech_client.rs`、`llm_client.rs`、`http_error.rs`、`tests/speech_client.rs`、`tests/llm_client.rs`、`tests/support/` 只服務 HTTP 路徑；`ureq`、`serde`、`serde_json` 只在這些檔案用到。`translator-models` 有自己的 `ureq`（含 TLS），不受影響。
- native 函式庫的位置是 `<執行檔所在資料夾>/lib`（`translator-io/src/native/mod.rs:15-20`）；`make run-native` 用 symlink `target/debug/lib → native/<arch>/lib` 達成（`Makefile:47-50`）。
- Python 與 Node：`backend/`、`setup.sh`、`start.sh`、`download_model.sh`、`frontend/`、根目錄的 `package-lock.json`、`deploy-pi.sh`、`deploy/gemma-translator.service`（啟動 `start.sh --prod`，含 `Environment=XDG_RUNTIME_DIR=/run/user/{{UID}}`）。
- `make build-pi` 從來沒能用過：這台機器沒有 aarch64 target 與交叉編譯環境。
- Slint 的 kms 後端（`backend-linuxkms-noseat`）透過 `input` crate 連結 libinput ≥ 1.21；EGL/GLES 由 glutin 在執行時 dlopen（`libEGL.so.1`、`libGLESv2.so.2`），不是編譯期連結；字型透過 fontconfig 連結（`yeslogic-fontconfig-sys`）。
- Bookworm 桌面版的 ALSA `default` 由 `pipewire-alsa` 導向 PipeWire；PipeWire 只在使用者的 systemd 工作階段裡執行。
- `scripts/setup-env.sh`（`make setup`）目前也處理 venv、litert-lm 的 Gemma 匯入與 `frontend/node_modules`。
- `model_dir` 預設 `/var/lib/gemma-translator/models`；在 Mac 與 Linux 桌機上不可寫，目前靠使用者在設定檔指定。

## 3. 設計

### 3.1 移除 HTTP 路徑
- `translator-core`：刪除 `InferenceBackend` 與 `inference_backend`、`speech_service_url`、`llm_endpoint`、`llm_model` 四個欄位。舊設定檔裡的這些鍵在解析時被略過，下次存檔時消失。
- `translator-io`：刪除 `speech_client.rs`、`llm_client.rs`、`http_error.rs`、`tests/speech_client.rs`、`tests/llm_client.rs`、`tests/support/`；從 `Cargo.toml` 移除 `ureq`，以及刪除後不再使用的 `serde`、`serde_json`（以編譯結果為準）。
- `translator-app`：`assemble` 一律建立 native 服務；`--mock` 不變。
- 刪除 `backend/`、`setup.sh`、`start.sh`、`download_model.sh`、`frontend/`、根目錄的 `package-lock.json`、`deploy-pi.sh`、`deploy/gemma-translator.service`。
- `.gitignore`：保留 `dist/`；刪除只為已移除檔案存在的規則（`node_modules/`、`venv/`、`.venv/`、`env/`、`__pycache__/`、`*.py[cod]`、`.Python`、`server.log`、`/package-lock.json` 等，以實際檔案內容為準）。
- `venv/`、`frontend/node_modules` 不是 git 追蹤的檔案，不由腳本刪除，README 提醒使用者手動刪。

### 3.2 命令列參數 `--model-dir`
- `translator-app::cli::CliOptions` 新增 `model_dir: Option<PathBuf>`，`parse_args` 接受 `--model-dir <path>`，缺值時回錯誤 `--model-dir needs a path`。
- **不修改 `Config`**：`AppState::new` 與 `AppState::headless` 新增參數 `model_dir_override: Option<PathBuf>`，只傳給 `native_services`，由它決定 `ModelPaths` 用覆蓋值還是 `config.model_dir`。`Controller` 拿到的 `Config` 與存檔內容完全不受影響。
- `translator` 與 `translator-tui` 把 `CliOptions::model_dir` 傳給 `AppState::new`。
- `--mock` 時忽略 `--model-dir`。

### 3.3 Makefile 與 `make setup`
- `make test` 只跑 `cargo test --workspace`。
- `make run`：依賴 `native-libs`；先 build `translator`，在執行檔資料夾建立 `lib → native/<arch>/lib` 的 symlink（沿用現有 `run-native` 的做法），再以 `--model-dir $(MODEL_DIR)` 執行。原本的 `run-native` 併入 `run` 後刪除。
- `make run-tui`：同樣依賴 `native-libs`、建立 symlink、帶 `--model-dir $(MODEL_DIR)`（`ARGS` 照舊附加在後面）。兩個執行檔在同一個 `target/debug/`，共用同一個 symlink。
- `run-mock` 不變。`MODEL_DIR` 預設仍為 `$(CURDIR)/models`。
- `make package-pi` 取代 `build-pi`（見 3.4）。
- `scripts/setup-env.sh`：
  - 系統工具檢查移除 `python3` 與 `npm`；Linux 新增 `pkg-config --exists fontconfig`（缺少時提示 `sudo apt-get install -y libfontconfig1-dev`）。其餘不變（Linux：`make`、`patchelf`、`pkg-config`、ALSA 開發檔、`libvulkan.so.1`；macOS：Apple Silicon、macOS 15+、Xcode CLT；共同：`cargo`、`curl`、`unzip`、`sha256sum` 或 `shasum`）。
  - 專案項目只剩 native 函式庫與模型；移除 venv、Gemma 匯入、`node_modules` 三項。
  - 完成提示改為 `make run` / `make run-tui`。

### 3.4 Pi 打包：`make package-pi`
只在 x86_64 Linux 上執行；其他平台以明確訊息結束。

1. **建置環境**：`cross` 固定使用 0.2.5（`cargo install cross --version 0.2.5 --locked`）。cross 預設的 aarch64 映像檔不能用：0.2.5 是 Ubuntu 16.04（libinput 太舊），開發版是 Ubuntu 24.04（glibc 2.39，超過 Pi 的 2.36）。所以 repo 新增 `deploy/Dockerfile.pi`，以 `debian:bookworm` 為基底（glibc 2.36、libinput 1.22，與 Pi 相同）：
   - `dpkg --add-architecture arm64`，安裝 `crossbuild-essential-arm64`、`pkg-config`，以及 arm64 開發套件 `libasound2-dev:arm64`、`libinput-dev:arm64`、`libudev-dev:arm64`、`libgbm-dev:arm64`、`libdrm-dev:arm64`、`libxkbcommon-dev:arm64`、`libfontconfig1-dev:arm64`（完整清單以實際編得過為準）。
   - 設定 `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc`、`CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc`、`CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++`、`PKG_CONFIG_ALLOW_CROSS=1`、`PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig`。
   - repo 新增 `Cross.toml`：`[target.aarch64-unknown-linux-gnu] dockerfile = "deploy/Dockerfile.pi"`。
   - 主機需要 Docker 與 `rustup target add aarch64-unknown-linux-gnu`；缺 `cross`、Docker 或 target 時，列出安裝指令後結束。
2. **交叉編譯**：`cross build --release -p translator-ui --no-default-features --features kms --target aarch64-unknown-linux-gnu`。
3. **native 函式庫**：`scripts/fetch-native-libs.sh aarch64`，放在 `native/aarch64/lib/`（已有 `.complete` 就跳過）。
4. **符號版本檢查**：用 `readelf --dyn-syms`（不依賴主機 binutils 是否支援 aarch64）讀出 `translator` 與 `native/aarch64/lib/*.so` 需要的最高版本：`GLIBC_` 不得超過 2.36，`GLIBCXX_` 不得超過 3.4.30（Bookworm 的 libstdc++）。超過就結束並列出是哪個檔案、哪個版本。
5. **模型檢查**：`$(MODEL_DIR)/.complete` 必須存在（`make models` 寫的標記），否則提示先執行 `make models`。
6. **打包**：先寫到暫存檔，成功後改名為 `dist/gemma-translator-pi.tar`（不壓縮，約 4.7 GB：模型約 4.5 GB、函式庫約 190 MB）。內容為頂層資料夾 `gemma-translator/`：

   | 路徑 | 來源 |
   |---|---|
   | `translator` | 交叉編譯的執行檔 |
   | `lib/` | `native/aarch64/lib/` 內的 `.so` 檔（不含 `.complete`） |
   | `models/` | `$(MODEL_DIR)` 的內容，排除 `cache/` 與 `.gitignore`；`.complete` 一併打包 |
   | `gemma-translator.service` | `deploy/gemma-translator.service.in` |
   | `install.sh` | `deploy/install.sh` |

   `cache/` 是 litert-lm 依機器產生的編譯快取，不打包；Pi 第一次啟動會重建，所以第一次開機到可以使用會比較久。

### 3.5 Pi 安裝：`sudo ./install.sh`
放在 `deploy/install.sh`，打包時複製進 tar。必須以 `sudo` 執行；服務使用者為 `$SUDO_USER`（沒有或為 root 則結束並說明），`UID` 由 `id -u "$SUDO_USER"` 取得。

**檢查階段**（全部通過前不修改系統）：
1. `uname -m` 為 `aarch64`；`ldd --version` 的 glibc ≥ 2.34。
2. **執行時套件**：用 `dpkg -s` 檢查固定清單：`libegl1 libgles2 libgbm1 libgl1-mesa-dri libvulkan1 mesa-vulkan-drivers libinput10 libxkbcommon0 xkb-data libudev1 libasound2 libfontconfig1 libdrm2 pipewire pipewire-alsa wireplumber alsa-utils fonts-dejavu-core fonts-noto-cjk fonts-noto-core`。這些大多是執行時 dlopen 或資料檔，`ldd` 看不到。
3. **連結檢查**：對 `translator` 與 `lib/*.so` 執行 `ldd`（`LD_LIBRARY_PATH` 指向套件的 `lib/`），收集 `not found` 的函式庫；已知的用對照表轉成套件名，未知的原樣列出。
4. 第 2、3 步有缺少時，一次列出 `sudo apt-get install -y …`（並註明這一步需要網路）後結束。腳本不呼叫 apt。

**安裝階段**（依序）：
5. **停止舊服務**：`systemctl stop gemma-translator.service`（不存在就略過）。舊版是 `start.sh --prod`，會占用數 GB 記憶體並設有 `Restart=always`，必須先停。
6. **清理殘留**：刪除前一次失敗留下的 `/opt/gemma-translator.new/`、`/opt/gemma-translator.old/`。
7. **安裝模型**（只新增或覆蓋，不刪除）：複製到 `/var/lib/gemma-translator/models/`，大小與修改時間相同的檔案跳過（有 `rsync` 用 `rsync -a`，否則逐檔比較大小與修改時間後 `cp -p`）；建立 `cache/`；整個資料夾擁有者設為服務使用者。
8. **安裝程式**：複製 `translator` 與 `lib/` 到 `/opt/gemma-translator.new/`；把現有的 `/opt/gemma-translator/` 改名為 `.old`，再把 `.new` 改名為正式名稱，最後刪除 `.old`。
9. **systemd 服務**：把範本的 `{{USER}}`、`{{UID}}` 換掉，寫入 `/etc/systemd/system/gemma-translator.service`。範本內容：
   - `[Unit]`：`After=sound.target user@{{UID}}.service`、`Wants=user@{{UID}}.service`、`Conflicts=getty@tty1.service`、`After=getty@tty1.service`
   - `[Service]`：`ExecStart=/opt/gemma-translator/translator`、`User={{USER}}`、`SupplementaryGroups=video input render audio`、`Environment=XDG_RUNTIME_DIR=/run/user/{{UID}}`、`Restart=always`、`RestartSec=2`
   - `[Install]`：`WantedBy=multi-user.target`
10. **音訊工作階段**：`loginctl enable-linger "$SUDO_USER"`，讓使用者的 systemd 工作階段（含 PipeWire、WirePlumber）在沒有人登入時也會在開機時啟動。這樣 ALSA `default` 與 `wpctl` 跟驗收時桌面環境下的行為一致。
11. **改開機方式**：`systemctl set-default multi-user.target`，讓桌面不佔用 DRM（Slint 以 `video` 群組成員的身分第一個開啟 DRM 裝置即取得 master）。服務設定 `Conflicts=getty@tty1.service`，執行時 tty1 不會有登入提示，按鍵也不會送到 shell；服務停用後 tty1 的登入提示自然恢復。若服務使用者的 `~/.config/lxsession/rpd-x/autostart` 有 `@chromium` 行（舊 `deploy-pi.sh` 寫的），就刪掉。
12. **啟動**：`systemctl daemon-reload`、`systemctl enable gemma-translator.service`、`systemctl restart gemma-translator.service`（`restart` 確保更新時換成新版）。最後印出「重開機後會直接進入翻譯畫面」與改回桌面開機的指令 `sudo systemctl set-default graphical.target`。

設定檔沿用服務使用者的 `~/.config/gemma-translator/config.toml`（systemd 對 `User=` 服務會設定 `HOME`）；`~/.config/gemma-translator/` 必須由服務使用者擁有。`model_dir` 預設值 `/var/lib/gemma-translator/models` 與安裝位置一致，不需修改。

### 3.6 macOS 與 Linux 桌機
- 開發與日常使用：`make setup`、`make run`、`make run-tui`、`make test`、`make test-ffi`。模型在 repo 的 `models/`，透過 `--model-dir` 傳入；函式庫透過 `make run`/`make run-tui` 建立的 symlink 載入。
- Mac 不支援 `make package-pi`。

### 3.7 文件
- README：
  - 重寫安裝與執行：開發機用 `make setup` + `make run`；Pi 用 `make package-pi` + `sudo ./install.sh`，註明支援的 Pi 系統、tar 約 4.7 GB、第一次開機較慢。
  - 刪除 Python、React、`start.sh`、`deploy-pi.sh` 的說明，以及 HTTP / native 兩種後端的區分。
  - 快捷鍵章節裡關於瀏覽器 `localStorage` 的說明，改成設定頁與 `config.toml`。
  - 提醒舊使用者手動刪除 `venv/` 與 `frontend/node_modules`。
- CLAUDE.md：Overview 的「三個程序」表格、Commands（`run-native` 改為 `run`、新增 `package-pi`）、Architecture 改寫成單一 Rust 程式；移除 Python / React 相關段落與 HTTP 後端、`inference_backend` 的說明。
- `docs/superpowers/specs/2026-10-09-rust-frontend-design.md` 第 11 節：標註階段 A 步驟 6 的部署切換與步驟 11 由本文件完成。
- `docs/superpowers/specs/2026-10-09-tui-frontend-design.md`：成功標準裡的三種模式改為 `--mock` 與 native。

## 4. 錯誤處理
- 舊設定檔含已刪除的鍵：略過，不報錯。
- `--model-dir` 指向不存在的資料夾：沿用現有 native 服務的錯誤（例如 `model file … not found; run translator-models`），畫面顯示錯誤狀態，程式不結束。
- `make package-pi` 在非 x86_64 Linux、缺 `cross`/Docker/rustup target、符號版本超過上限、模型未完成時：明確訊息，非 0 結束，不產生半成品 tar。
- `install.sh` 非 root、沒有 `SUDO_USER`、非 aarch64、glibc 太舊、缺套件或函式庫：明確訊息，非 0 結束，且在這之前不修改系統（檢查階段在安裝階段之前）。
- 安裝階段中途失敗：
  - 第 7 步（模型）失敗：`/opt/gemma-translator/` 還是舊版；模型資料夾只新增或覆蓋過檔案，不會少檔。重新執行 `install.sh` 即可。
  - 第 8 步改名之間失敗：`/opt/gemma-translator.old/` 保留舊版；重新執行時第 6 步會清掉殘留，再完整安裝一次。
  - 舊服務已在第 5 步停止，失敗後不會自動恢復；重新執行 `install.sh` 或 `sudo systemctl start gemma-translator` 即可。

## 5. 測試
- Rust：
  - `cli::parse_args`：`--model-dir /x` 設定值；缺值時回錯誤；與 `--mock`、`--config` 並用。
  - 覆蓋只影響執行期：以 `AppState::headless` 帶 `model_dir_override` 建立、觸發存檔後，寫出的 `model_dir` 是設定檔原本的值。
  - `Config::load`：含 `inference_backend`、`speech_service_url`、`llm_endpoint`、`llm_model` 的舊設定檔能載入，其他欄位正確。
  - 修改或刪除引用已刪除欄位的現有測試：`config.rs` 的 `partial_file_fills_in_defaults`（`llm_model`）與 `inference_backend_and_model_dir_have_defaults_and_parse`。
  - 刪除 HTTP 客戶端與其測試後，`cargo test --workspace`、`make clippy`、`cargo fmt --all -- --check` 全過。
- Shell：`bash -n` 檢查 `deploy/install.sh`、`scripts/setup-env.sh`；在這台機器上實際執行 `make setup`（全部 `ok`）、`make run-mock`（可啟動）與 `make package-pi`（編譯成功、符號版本檢查通過、`tar -tf` 的內容符合 3.4 的表格）。
- 交叉編譯產物：`file` 顯示 `ARM aarch64`；`readelf -d` 的 `NEEDED` 不含 `libEGL`/`libGLESv2`（確認仍為 dlopen）。

## 6. 手動驗收
### Pi 5（使用者，Raspberry Pi OS Bookworm 64 位元桌面版）
1. 複製 tar 並執行 `sudo ./install.sh`：檢查通過後完成安裝；舊的 `start.sh` 服務已停止。
2. 重開機：不進桌面，直接進入翻譯畫面；tty1 沒有登入提示。第一次啟動等待 litert-lm 重建快取。
3. 完整翻譯一輪：錄音、辨識、翻譯、播放；音量鍵能調整音量（PipeWire / `wpctl`）。
4. 用新的 tar 再安裝一次：模型不重複複製，服務重啟成新版。
5. `sudo systemctl disable gemma-translator && sudo systemctl set-default graphical.target` 後重開機可回到桌面。

### Mac（使用者）
1. `make setup`、`make run`、`make run-tui` 各完整翻譯一輪。
2. `make test`、`make test-ffi` 全過。
