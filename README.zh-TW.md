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

視窗標題包含啟動目錄標籤、pane ID 與括號標籤；程式提供標題時，再接上該標題：

```text
{status} {dir} · {pane_id} ({role}, herdr-pid: {pid}, cluster: {pid}) — {program title}
```

括號標籤顯示 agent 名稱、`herdr-pid` 與 `cluster`。`herdr-pid` 是啟動目錄的 herdr server 的 process ID，同一目錄開出的每個視窗都相同，包括 agent 建立 pane、tab 或 workspace 時開出的 switch shell 視窗。server 閒置後自行結束時，下一次啟動會開新的 server，之後啟動的視窗就顯示新的 process ID。`cluster` 是 cluster 的 process ID，只出現在 `cluster` 開的視窗，和 cluster 的結束訊息、timeout log 檔名與 `kill <pid>` 是同一個數字。用不到的部分不顯示，全部都沒有時，整個括號也不顯示。

💤 閒置 · ⏳ 工作中 · ✅ 完成 · ✋ 等待回應。狀態未知或沒有 agent 時不顯示圖示。**完成**表示工作已結束，會保持顯示直到下一個任務開始。

## Cluster

在 Unix 上，`cluster` 可從同一個 canonical directory（解析符號連結後的目錄）啟動一或多個 agent，讓它們共用一個 switch session：

```bash
easy-herdr cluster claude codex opencode-local "reply with one word: ready"
easy-herdr cluster claude codex -- hello
```

第一個 agent 在目前終端機執行。其餘 agent 會優先在目前終端機應用程式開新分頁；分頁指令失敗時，改開新視窗：

| 平台 | 支援分頁的終端機 | 其他終端機 |
| --- | --- | --- |
| Linux | Konsole、kitty、WezTerm、GNOME Terminal | 開新視窗，包含 Ghostty |
| macOS | kitty、WezTerm、iTerm2 | 開新視窗，包含 Terminal.app 與 Ghostty |

kitty 需要啟用 remote control 並設定監聽 socket（`listen_on`，由 `KITTY_LISTEN_ON` 環境變數提供），才能開分頁；沒有 socket 時會開新視窗。只有 `HERDR_SWITCH_TERMINAL` 含非空白指令時，才使用你指定的視窗指令。

可選的 prompt 只會送給第一個 agent。easy-herdr 每 250 ms 檢查一次，偵測到 agent 為 idle 或 done 後，才透過 herdr API 輸入並送出 prompt，最多等待 120 秒。若啟動時出現授權對話框，請在等待期間自行回答。prompt 不會放進 agent 的命令列參數。

- 沒有 `--` 時，最後一個參數若包含空白或上述 agent 名稱字元以外的字元，且不以 `-` 開頭，就視為 prompt。合法的單字名稱一律視為 agent，找不到就報錯。
- 單字 prompt 請放在 `--` 後，也可在其後輸入多個單字，程式會用空白串成 prompt；`--` 後不能留空，prompt 也不能是空字串或只有空白。
- 開啟任何終端機前，會先驗證所有 agent。此指令不接受 agent 選項；完整用法請看 `easy-herdr cluster --help`。

### 讓 cluster 做完自己收尾

預設情況下，任務做完後 cluster 的每個 agent 都會一直開著，要你自己關視窗。加上 `--exit-when-done`，cluster 會自己收尾；加上 `--background`，它還能完全不開視窗，適合只能用 SSH 連線的主機。不加這些選項時，cluster 的行為和以前完全一樣。

可以把它想成會議室預約：大家安靜一段時間後就歸還會議室，最晚也會在預約時間到時歸還。

```text
啟動 ──► 有 agent 工作過 ──► 所有 agent 都 idle 滿 --idle-exit（5m）──► 關閉 pane，exit 0
啟動 ──► 到了 --timeout（2h）還沒結束 ──► 照樣關閉 pane，exit 124
```

至少要有一個 agent 工作過，才開始計算 idle 時間；期間只要又有 agent 開始工作，就重新計時。cluster 只關閉自己開的 pane，同目錄的其他 agent 不受影響。沒有別人使用時，herdr server 約 5 秒後會自己結束。

| 選項 | 不加時 | 加上後 |
| --- | --- | --- |
| `--exit-when-done` | 任務做完後，視窗與 agent 一直開著，要自己關。 | 所有 agent 都 idle 滿 `--idle-exit` 後，關閉 cluster 的所有 agent pane。前景模式接著印一行訊息後 exit 0。 |
| `--timeout <D>` | 預設 `2h`，只在搭配 `--exit-when-done` 或 `--background` 時有作用。 | 超過 D 還沒結束，就照樣關閉 pane。前景模式印出 `timed out after D` 並 exit 124；背景模式改在目前目錄寫 log。 |
| `--idle-exit <D>` | 預設 `5m`，規則同上。 | 設定所有 agent 要 idle 多久才算完成。 |
| `--background` | 第一個 agent 用目前的終端機，其他 agent 開新分頁或新視窗。 | 不開任何視窗，所有 agent 在看不見的終端機裡執行；指令印出 process ID 後立刻以 exit 0 返回，終端機可以繼續用，也可以直接登出。自動包含 `--exit-when-done`，而且一定要給 prompt。 |

`D` 是正整數加上 `s`、`m` 或 `h`，例如 `30s`、`90m`、`2h`。選項一律寫在第一個 agent 名稱前面，因為 agent 後面的最後一個參數會被當成 prompt。

看到什麼、exit code 是多少，取決於 cluster 怎麼執行：

| 行程 | 輸出 | exit code |
| --- | --- | --- |
| 前景 cluster | 在 stdout 印一行結束訊息 | 完成或所有 agent 都結束時為 0，timeout 為 124，失去 herdr server 時為 1 |
| 背景 launcher（你輸入的指令） | 四行啟動訊息 | 啟動後立刻 exit 0 |
| 背景 supervisor（脫離終端機的行程） | 不輸出，因為 stdout 導到 `/dev/null`；timeout 時寫 timeout log | timeout 為 124，`kill` 後為 143，啟動步驟失敗為 1，其他為 0 |

### 哪些選項可以一起用

`--background` 已經包含 `--exit-when-done`，兩個都寫不會出錯，只是多餘。背景模式不能關掉自動收尾：沒人看著它，不收尾的話 agent 會一直跑下去。

| 選項 | 允許 | 結果 |
| --- | --- | --- |
| （無） | 可以 | 和以前相同 |
| `--exit-when-done` | 可以 | 前景，timeout 2h，idle 5m |
| `--exit-when-done --timeout 3h --idle-exit 10m` | 可以 | 前景，timeout 3h，idle 10m |
| `--background` | 可以 | 背景，timeout 2h，idle 5m |
| `--background --exit-when-done` | 可以 | 與上一列相同 |
| `--background --timeout 4h` | 可以 | 背景，timeout 4h；timeout 時寫 log |
| `--background --exit-when-done --timeout 4h --idle-exit 10m` | 可以 | 背景，timeout 4h，idle 10m |
| `--timeout 1h` 單獨使用 | 不行，exit 2 | `--timeout requires --exit-when-done or --background` |
| `--idle-exit 10m` 單獨使用 | 不行，exit 2 | `--idle-exit requires --exit-when-done or --background` |
| `--background` 沒給 prompt | 不行，exit 2 | `--background requires a prompt` |
| 同一個選項寫兩次，例如 `--timeout 1h --timeout 2h` | 不行，exit 2 | `--timeout given twice` |

用法錯誤時不會開任何視窗。

### cluster 使用範例

```bash
# 不用新選項：和以前一樣
easy-herdr cluster claude claude claude codex -- "You are the planner. ..."

# 前景，做完自動關閉（timeout 2h、idle 5m 都用預設）
easy-herdr cluster --exit-when-done claude claude claude codex -- "You are the planner. ..."

# 前景，最多跑 3 小時，全部 idle 10 分鐘才算完成
easy-herdr cluster --exit-when-done --timeout 3h --idle-exit 10m claude claude claude codex -- "..."

# 背景（只有 SSH 的主機）：立刻返回，timeout 用預設 2h
easy-herdr cluster --background claude claude claude codex -- "..."

# 背景，自訂 timeout
easy-herdr cluster --background --timeout 4h claude codex -- "..."
```

背景啟動時會印出查看與停止的方法：

```text
easy-herdr cluster 12345 started in the background
watch: easy-herdr --session sw-easy-herdr-1a2b3c4d5e6f agent list
stop:  kill 12345
log:   ./easy-herdr-cluster-*-12345.log is written only if it times out
```

前景 cluster 結束時，會在原本的終端機印一行訊息：

```text
easy-herdr cluster 12345: all agents idle for 5m; closed 4 agent panes
easy-herdr cluster 12345: timed out after 2h; closed 4 agent panes
```

`kill <pid>` 會乾淨地停止背景 cluster：關閉它的 pane、清掉工作用的檔案，不寫 log。timeout log 的檔名是 `./easy-herdr-cluster-<YYYYMMDD-HHMMSS>-<pid>.log`，時間取本地的啟動時間；內容有各 agent 的狀態、狀態變化的時間軸，以及每個 agent 畫面的最後 200 行。

### cluster 的視窗標題

cluster 開的每個視窗都會以 `herdr-pid` 顯示 herdr server 的 process ID，並以 `cluster` 顯示 cluster 的 process ID；agent 有名稱後，也會顯示角色：

```text
easy-herdr · w1:p1 (planner, herdr-pid: 39001, cluster: 12345)
```

角色就是 herdr 的 agent 名稱。cluster 視窗的角色來自 `easy-herdr agent rename <pane> <name>`，例如 planner 替自己改名；改名後約 2 秒內，標題就會更新。`easy-herdr switch <cli> <name>` 則是替一般的 switch 視窗命名，這種視窗顯示 `(name, herdr-pid: <pid>)`，沒有 `cluster`。

### 使用自動收尾前要知道的事

自動收尾可能誤判的情況：

- 「所有 agent 都 idle」也可能是 planner 停下來等你回答，`--idle-exit` 就是緩衝時間。
- Herdr 一直偵測不到的 agent CLI，會讓 cluster 一直跑到 `--timeout`。

背景模式的限制：

- 沒有人能回答 agent 的啟動或核准對話框。卡住的 agent 會顯示為 blocked，最後在 `--timeout` 時結束並留下 log。
- agent 若要求新增 pane、tab 或 workspace，easy-herdr 仍會嘗試開原生視窗，在沒有 GUI 的主機上會失敗。
- 需要可寫入的工作目錄來放 timeout log；不可寫入時，指令會立刻以狀態碼 1 結束。

出問題時去哪裡看：

- 背景模式的診斷訊息（例如啟動中止、log 寫入失敗）寫在 session 設定目錄的 `herdr-cluster.log`：`~/.config/easy-herdr/sessions/<session>/herdr-cluster.log`（debug 版為 `easy-herdr-dev`）。
- systemd-logind 若設定 `KillUserProcesses=yes`，登出時會結束 SSH session scope 裡的所有行程，包括從那裡啟動的背景 cluster。只執行 `loginctl enable-linger` 不夠：它只讓你的 user manager 繼續執行，cluster 還必須在 session scope 之外執行。例如在啟用 linger 後，於 user manager 的 scope 裡啟動：`systemd-run --user --scope easy-herdr cluster --background …`。說明請見 [`KillUserProcesses=`](https://www.freedesktop.org/software/systemd/man/latest/logind.conf.html#KillUserProcesses=)。

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
