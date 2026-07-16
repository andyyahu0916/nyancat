# Roadmap

這份文件記錄 rustification 進入主線後的後續方向，也保留已完成階段的摘要。新的工程工作以這份 roadmap、`ARCHITECTURE.md` 和 `RELEASE_CHECKLIST.md` 為準。

## Current State

- `master` 是目前主線，已包含 rustified core。
- `legacy/split-main` 保留 `3544fa8`，作為純拆分後、rustification 前的 baseline。
- `refactor/split-main` 和 `refactor/rustify-core` 是歷史開發分支，可等穩定發行後再決定是否刪除。
- Rustification 階段已取得階段性勝利；後續重心轉向發行品質、可驗證效能、以及更精緻的模組邊界。

## Documentation Map

- `README.md`：使用者入口、基本建置、執行、telnet、benchmark 方法。
- `ARCHITECTURE.md`：目前模組邊界、資料流、runtime policy、performance policy、extension guidelines。
- `RELEASE_CHECKLIST.md`：merge / release candidate 前的硬性驗證流程，以及 benchmark snapshot 紀錄。
- `ROADMAP.md`：後續工程方向與優先順序。

## Completed Rustification

第一階段目標是擺脫「披著 Rust 外皮的 C-style 結構」，同時保留既有 terminal output 行為與 hot path 效能。已完成的核心成果：

- 拆分 `main.rs`，將 CLI、render、runtime、sys、telnet、terminal、animation 分成獨立模組。
- CLI 改成 `CliAction` / `CliError`，`OPTION_SPECS` 成為 parser 和 `--help` 的單一資料源。
- `Config` 逐步型別化：`FrameLimit(NonZeroU32)`、`Duration` delay、`AxisCrop` / `AxisRange` 取代 frame/crop magic values。
- terminal、palette、frame symbol、terminal size 改成語意型別；`TerminalSize` 內部保證 bounded positive dimensions，render hot path 保留 O(1) palette lookup。
- `RenderState`、`Renderer`、`render/` 子模組分離 frame bytes 生成、palette lookup、timing、buffer reuse、telnet newline 和 benchmark accounting。
- telnet negotiation 拆成 parser、state machine、subnegotiation parser 和 `ByteSource`；command / option 已型別化，未知 option 仍以 raw byte newtype 保留並可測。
- 遠端 NAWS 已在 protocol boundary 加入寬、高與 viewport area budget；mid-session poll/read error 不再一律吞成正常斷線。
- `TerminalSession` 用 RAII restore terminal；Unix FFI 和 signal path 集中在 `sys.rs` / `runtime.rs`，stdin poll/read 回傳 typed outcomes 而不是吞成 bool / `Option`。
- release gate、output smoke golden checks、真實 PTY resize、loopback telnet integration、benchmark report、benchmark matrix 和 CI/MSRV/macOS jobs 已建立。

仍保留的條件式方向：

- app-level error enum 只有在 runtime / telnet / CLI 錯誤語意真的需要跨模組統一時才做。
- 更大幅的 render / telnet / sys 重構必須先用 release gate 和 benchmark 保護行為。

## Near-Term Plan

### 1. Release Hardening

目標是讓專案從「可用」變成「可以放心發行」。

- 保持 `scripts/release_check.sh` 作為 release gate。
- 讓 release checklist 覆蓋 clean checkout、tagging、artifact、manpage、systemd files。
- 維護 GitHub Actions CI，讓 stable Rust 跑 release check，MSRV job 跑 Rust 1.85.0 test / release build。
- 發行前用 `scripts/benchmark_matrix.sh` 更新 `RELEASE_CHECKLIST.md` 的 benchmark snapshot。

完成標準：

- clean working tree 可以一鍵跑完 `scripts/release_check.sh`。
- release tag 前的人工步驟都能在 `RELEASE_CHECKLIST.md` 找到。
- benchmark 宣稱都能回溯到 `RELEASE_CHECKLIST.md` 的環境與 commit。

### 2. Output Regression Coverage

目前 smoke test 已用 committed golden files 做 exact output comparison，並保留關鍵輸出 marker。golden comparison 提供 deterministic output regression guard；局部 marker 則讓錯誤訊息能指出主要語意差異。

- 保留 golden smoke，因為它便宜且能抓到完整輸出漂移；golden 檔案本身包含 terminal output 的空白與控制碼，更新時必須 review diff。
- 維護 release smoke 的關鍵 marker 檢查，覆蓋 xterm / truecolor / telnet newline / no-counter 行為。
- 保留真實 controlling PTY 的 SIGWINCH reflow smoke，以及以 ephemeral loopback TCP 驗證 NAWS resize / orderly disconnect 的 integration tests。
- 若未來 golden fixture 太脆弱，再補更細的 semantic output verifier。

完成標準：

- render 或 terminal-output 變更可以清楚說明「哪些輸出變了，為什麼合理」。
- release check 能抓到 help / CLI / benchmark / smoke output 的明顯漂移。

### 3. Modern Rust Hardening

目標不是「為了 Rust 而 Rust」，而是務實地處理 C-era 玩具程式常見的 trade off：magic number、sentinel state、raw byte protocol、process-global state、以及靠人工紀律維持的不變條件。每次改動都要能說明它讓程式更安全、更快、或更容易驗證。

- 優先消滅 internal sentinel：用 `Option`、`NonZero*`、newtype、enum 表達狀態，而不是讓 `0`、`-1`、裸整數跨模組傳遞語意。
- 將 public CLI 相容需求限制在 CLI 邊界內；進入 render/runtime/telnet 後應該是已驗證、型別化的設定。
- 持續收斂 protocol/raw byte 邊界，保留未知 telnet option 的 pass-through 能力。
- telnet parser 已有 in-tree 對抗測試（`parser_survives_adversarial_input`，零依賴 PRNG + 針對性種子，跑在 `cargo test` gate 內，斷言不 panic / sb buffer 有界 / 解析具決定性 / 協商必定終止）。更深的覆蓋（nightly `cargo-fuzz` libFuzzer target）列為未來延伸，需 nightly toolchain，不進預設 gate。
- 強化 Unix FFI 邊界：優先補上 EINTR/error handling 與更清楚的 safe wrapper；若可攜性收益明確，再評估 `libc` / `sigaction` / `signal-hook`。
- 將 render hot path 的改動建立在 benchmark 上；型別化不得引入 per-cell allocation、dynamic dispatch、或高頻 format work。

完成標準：

- 重要 invariants 由型別或建構函式保證，而不是只靠註解或呼叫端紀律。
- legacy compatibility 只留在 parsing/adaptation 層，核心模組不直接依賴 legacy sentinel。
- 每個 safety/refactor commit 都能通過 release gate；效能相關 commit 需重跑 benchmark matrix。

### 4. Performance Discipline

效能優化要建立在可重跑 benchmark 和明確瓶頸上。

- 先 profile，再改 hot path。
- 效能驗證以 `scripts/benchmark_callgrind.sh` 的確定性指令數為準（callgrind 計數 frequency-independent，免疫變頻 / 排程雜訊）。`scripts/benchmark_matrix.sh` 的 wall-clock FPS 只當輔助：本機是 asusctl + scx_LAVD 筆電，throughput 可跨 run 漂移約 2x，單次 wall-clock 對小改動不可信。
- 優先觀察 frame buffer reuse、palette lookup、newline conversion、counter formatting、stdout write pattern。
- 避免為了抽象引入 per-cell allocation、dynamic dispatch、或高頻 format work。

完成標準：

- 每個 performance commit 都能附上「變更前 / 變更後」的 callgrind 指令數對照。
- 若效能沒有明顯改善，保留可讀性收益時才接受。

### 5. Module Elegance

目標不是把檔案切碎，而是讓每個模組的責任更明確。

- `cli.rs` 已有 `OPTION_SPECS`，目前用測試防止 README / manpage option drift；後續若文件格式穩定，可再考慮由同一份資料生成 option table。
- `render.rs` 已拆出 `palette`、`frame_buffer`、`render_loop`、`benchmark` 這些自然邊界；後續只有在 `Renderer` / `RenderState` 長出新責任時再繼續拆。
- `telnet.rs` 目前可測性已足夠，除非新增協定行為，否則不急著拆。
- app-level error enum 仍是條件式保留項，只有錯誤語意跨模組變複雜時才做。

完成標準：

- 拆分後測試覆蓋不下降。
- public behavior 和 smoke golden output 沒有非預期變化。
- hot path 不因為拆分而變慢。

### 6. Packaging And Distribution

這部分會把終端玩具推向正式軟體發行品質。

- `v*` tag 會觸發 `.github/workflows/release.yml`：workflow 先驗證 tag 等於 `v${Cargo.toml version}`，Linux 與 macOS 平行建 archive；兩者都成功後才產生 `SHA256SUMS`、重建未完成的 draft 並發布 immutable GitHub Release。已發布版本不允許 workflow 覆寫 live assets。目前產出 Linux 與 macOS host-target artifact，更多 target（其他 arch / BSD）是後續 build matrix 延伸。
- crates.io 發佈仍是「刻意延遲」的決策，`Cargo.toml` 以 `publish = false` 防止誤發。若未來要發布，需先確認 crate 名稱所有權與 token，再以明確 commit 解除此保護。
- 將 package manifest 檢查納入 release gate，避免發行檔案清單或 metadata 在最後一刻才出問題。
- 確認 manpage、systemd service、README 安裝步驟一致。
- 如果要改 default branch 名稱，先同步本機、GitHub default branch、README 文件。

完成標準：

- 新使用者能從 clean checkout 按 README 建置並跑起來。
- 發行者能照 `RELEASE_CHECKLIST.md` 完成 tag / artifact / benchmark / changelog。

## Decision Rules

- 先用 `RELEASE_CHECKLIST.md` 保護行為，再做結構或效能變更。
- 沒有 benchmark 就不宣稱效能進步。
- 沒有輸出驗證就不做大幅 render refactor。
- 新文件要放進 `Documentation Map`，避免散落成無人維護的筆記。
