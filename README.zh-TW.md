[English](README.md) · [繁體中文](README.zh-TW.md)

# easy-herdr

<p align="center">
  <img src="assets/herdr.svg" alt="easy-herdr 公羊吉祥物圖示" width="100" />
</p>

<p align="center">
  <img src="assets/mascot.png" alt="easy-herdr 公羊吉祥物" width="960" />
</p>

easy-herdr 是 [herdr](https://github.com/herdrdev/herdr) 的 fork。它保留 herdr 的所有功能，並新增**每個 agent 各用一個原生終端機視窗的 switch 模式**，支援 Linux 與 macOS。

原有功能請參閱[固定於參考 commit 的上游 README](https://github.com/herdrdev/herdr/blob/143bf5fd4781d164260c63fe28e7dc08054ad32f/README.md)。本機副本位於 `references/herdr`：

```bash
git submodule update --init references/herdr
```

## 安裝

從[此 fork 的 Releases](https://github.com/scott1028/herdr/releases) 下載符合平台的檔案：

| 平台 | 檔案 |
| --- | --- |
| Linux x86_64 | `easy-herdr-linux-x86_64`、`easy-herdr-linux-x86_64.AppImage` |
| Linux aarch64 | `easy-herdr-linux-aarch64`、`easy-herdr-linux-aarch64.AppImage` |
| macOS x86_64 | `easy-herdr-macos-x86_64` |
| macOS aarch64 | `easy-herdr-macos-aarch64` |
| Windows x86_64 | `easy-herdr-windows-x86_64.zip` |

Linux/macOS：將裸執行檔改名為 `easy-herdr`，用 `chmod +x easy-herdr` 加上執行權限，再放到 `PATH` 中。Windows：解壓縮 ZIP，讓 `easy-herdr.exe` 與 ConPTY 執行階段檔案保持在同一目錄，並將該目錄加入 `PATH`。

Unix 上，安裝 agent skill 與 hook 會呼叫的 `herdr` 指令：

```bash
easy-herdr install
```

這會在目前執行檔旁建立 `herdr` 符號連結。若要使用其他已存在且可寫入的目錄：

```bash
easy-herdr install --dir ~/.local/bin
```

若已有不同的 `herdr` 項目，會先改名為 `herdr.bak-<timestamp>`（UTC `YYYYmmddHHMMSS`）。既有備份絕不覆寫。符號連結若已指向同一執行檔，重跑不會更動它。Windows 不支援 `install`。

使用 AppImage 時，先讓下載的檔案具有執行權限，再執行其 `install` 指令，例如：

```bash
./easy-herdr-linux-x86_64.AppImage install
```

AppImage 需要 FUSE，或設定 `APPIMAGE_EXTRACT_AND_RUN=1`。當 `APPIMAGE` 指向現存檔案時，安裝會連結到該持久保存的映像檔，並預設使用其所在目錄，而非暫時掛載的執行檔。

## 快速開始

以下兩個指令等價：

```bash
easy-herdr codex
easy-herdr switch codex
```

語法：

```text
easy-herdr <agent-cli> [agent-args...]
easy-herdr switch <agent-cli> [agent-name] [-- agent-args...]
```

捷徑會把所有參數原樣傳給 agent CLI；緊接在 `<agent-cli>` 後的單一 `--` 會被去掉。例如，將 `MODEL` 換成要用的模型名稱：

```bash
easy-herdr codex --model MODEL
```

要在 herdr 內為 agent 命名，請用 `switch`。`agent-name` 不會傳給 agent CLI，agent 參數放在 `--` 後面：

```bash
easy-herdr switch codex reviewer -- --model MODEL
```

捷徑支援 `PATH` 上的可執行檔，以及由 `$SHELL` 指定之互動 shell 載入的 bash/zsh function 或 alias。名稱只能包含 ASCII 英文字母、數字、`.`、`_`、`+`、`-`，且不能以 `-` 開頭。既有 easy-herdr 指令優先。單純的內建指令與關鍵字不符合條件；找不到的名稱仍會印出 `unknown command: <name>` 並以狀態碼 2 結束。

## Switch 模式

從同一個正規化後的啟動目錄執行時，agent 共用一個 session，可以互相尋找與傳訊。不同啟動目錄使用不同 session。

在 switch session 中，agent 透過 herdr CLI 建立 pane、tab 或 workspace 時，每次建立都會開啟新的原生終端機視窗，在同一 session 執行 switch shell。關閉 switch 視窗會結束其 pane。

新視窗依目前終端機的環境變數選擇終端機：

| 平台 | 自動辨識的終端機 | 預設後備 |
| --- | --- | --- |
| Linux | Ghostty、Konsole、kitty、WezTerm、Alacritty、GNOME Terminal、foot | `x-terminal-emulator -e` |
| macOS | Ghostty、WezTerm、kitty、Alacritty、iTerm2 | Terminal.app |

以 `HERDR_SWITCH_TERMINAL` 覆寫開啟新視窗的指令：

```bash
HERDR_SWITCH_TERMINAL="kitty" easy-herdr codex
```

覆寫值是以空白分隔的指令前綴；easy-herdr 會在後面接上 switch 指令。前綴內不會解析 shell 引號。

新視窗會載入 bash 登入設定或 zsh 登入啟動檔。使用 bash/zsh 時，啟動後會將 `codex`、`claude`、`opencode` 的 shim 放回 `PATH` 最前面：

- Codex 會加上 `--no-daemon`，但若 `--` 前已有此參數、`--remote...` 選項，或指定 `agents`／`remote-control`，則不額外加入。
- OpenCode 的互動介面會加上 `--standalone`，使用私有 server。子指令及明確指定的 `--standalone`、`--no-standalone`、`--server` 選項維持原樣。
- Claude 保留原本的參數。

這讓預設的 Codex/OpenCode 工具呼叫使用啟動 session 的環境。

視窗標題包含啟動目錄標籤與 pane ID；程式提供標題時，再接上該標題：

```text
{status} {dir} · {pane_id} — {program title}
```

💤 閒置 · ⏳ 工作中 · ✅ 完成 · ✋ 等待回應。狀態未知或沒有 agent 時不顯示圖示。**完成**表示工作已結束，會保持顯示直到下一個任務開始。

## 與 herdr 的差異

- 執行檔名稱為 `easy-herdr`；`install` 為整合功能提供 `herdr` 符號連結。Switch 模式僅支援 Linux/macOS。
- Linux/macOS 的設定與 session 資料預設位於 `~/.config/easy-herdr`，狀態資料位於 `~/.local/state/easy-herdr`。Debug 建置使用 `easy-herdr-dev`。`XDG_CONFIG_HOME` 與 `XDG_STATE_HOME` 可覆寫基底目錄；Windows 使用 `APPDATA` 與 `LOCALAPPDATA`。
- 新目錄沒有 `config.toml` 時，只會從相同基底路徑下的舊目錄複製該檔案一次（正式版為 `herdr`，debug 版為 `herdr-dev`）。一般來源是 `~/.config/herdr/config.toml`。不遷移 session 或 plugin。
- 不會自行更新執行檔，也不執行背景版本檢查。`easy-herdr update` 與 `easy-herdr channel set` 會印出 [Releases 連結](https://github.com/scott1028/herdr/releases)，並以狀態碼 1 結束。`easy-herdr channel show` 仍會印出已設定的 channel，並以狀態碼 0 結束。Agent 偵測 manifest 更新仍依既有政策與設定執行。

## 從原始碼建置

使用 [`rust-toolchain.toml`](rust-toolchain.toml) 指定的 Rust 工具鏈（目前為 1.96.1），以及 Zig 0.16.0：

```bash
ZIG=/path/to/zig cargo build --release --locked
```

Linux/macOS 執行檔輸出為 `target/release/easy-herdr`。

## 發佈（維護者）

推送名稱為 `easy-herdr-v<Cargo version>` 的 tag，可選擇加上 `.<n>` 或 `-<suffix>` 後綴。版本必須符合 `Cargo.toml`。也可手動觸發 **easy-herdr release** workflow：

```bash
gh workflow run easy-herdr-release.yml --ref easy-herdr -f tag=easy-herdr-v0.9.5
```

## 授權

與上游相同，採用 Apache-2.0。請見 [LICENSE](LICENSE)。
