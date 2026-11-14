# Agent Duo v0.4 (Rust)

คุยกับทีม AI: เลือก **lead (orchestrator)** จะเป็น Claude, Codex หรือ AI เจ้าอื่นก็ได้ lead แบ่งงานให้ทีมตามที่คุณกำหนด agent ทุกตัว **แตก sub-agent ของตัวเองได้** คุณเห็นผังว่าใครแตกใครออกมา และดู **timeline** ได้ว่าใครทำงานช่วงไหน รอใคร ส่งข้อความหากันเมื่อไหร่

v0.4 คือเวอร์ชัน TypeScript/Next.js ที่เขียนใหม่ด้วย Rust มี 2 แบบ ใช้ engine ตัวเดียวกัน ไม่ต้องมี Node บนเครื่อง
- **แอป desktop (Tauri)** สำหรับ Windows และ macOS: ติดตั้งแล้วเปิดเป็นหน้าต่างของตัวเอง
- **`agent-duo` command-line** ไฟล์เดียว: ดับเบิลคลิกแล้วเปิดหน้าเว็บในเบราว์เซอร์

```
You ─▶ lead (Claude / Codex / DeepSeek …) ── propose_split ──▶ you approve
                    │
                    ├─ codex#1     build ──spawn_sub──▶ codex#1.a, codex#1.b
                    ├─ deepseek#2  tests
                    └─ claude#3    review   (after #1, #2)
```

## ติดตั้งแอป desktop

ดาวน์โหลดจากหน้า Releases
- **Windows**: `Agent Duo_x.y.z_x64-setup.exe` (หรือ `.msi`) ต้องมี WebView2 ซึ่งมีมากับ Windows 10/11 อยู่แล้ว
- **macOS 11+**: `Agent Duo_x.y.z_universal.dmg` (Apple Silicon และ Intel) ลากไปไว้ใน Applications
  - ถ้ายังไม่ได้ sign แอป macOS จะบอกว่าเปิดไม่ได้: คลิกขวาที่แอป → Open หรือรัน `xattr -dr com.apple.quarantine "/Applications/Agent Duo.app"`
  - แอปที่เปิดจาก Finder ได้ PATH สั้นมาก agent-duo จึงอ่าน PATH จาก login shell ของคุณ (`$SHELL -ilc`) และเพิ่ม `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin` ให้ เพื่อหา `claude`, `codex`, `node` และ `git`
  - session อยู่ที่ `~/Library/Application Support/agent-duo/runs`

แอปเปิด engine บน `127.0.0.1:47300` (ถ้าไม่ว่างจะหาพอร์ตอื่น) แล้วแสดงหน้าเดียวกับเวอร์ชันเบราว์เซอร์ในหน้าต่างของแอป ปิดหน้าต่างเพื่อหยุด agent ทุกตัว ตัวแอปเองเป็น `bridge` / `hook` ให้ agent เรียกด้วย

## เริ่มใช้ (command-line)

1. ดับเบิลคลิก `agent-duo.exe` หน้าต่าง console บอก URL และเปิดเบราว์เซอร์ให้เอง ปิดหน้าต่างนั้นเพื่อหยุด
2. เลือก **mock (no keys)** เพื่อลองดู flow ทั้งหมดทันที (ไม่แก้ไฟล์ใด ๆ) หรือ **real agents** แล้วใส่ path ของโฟลเดอร์โปรเจกต์
3. ถ้าจะใช้ agent จริง รัน `agent-duo doctor` ก่อน เพื่อเช็กว่า `claude`, `codex` และ `git` ติดตั้งและ login แล้ว

```
agent-duo                 เปิดหน้าเว็บ (พอร์ต 3000 ถ้าไม่ว่างจะหาพอร์ตอื่นให้)
agent-duo --port 4000     เลือกพอร์ตเอง
agent-duo --no-open       ไม่ต้องเปิดเบราว์เซอร์
agent-duo doctor          เช็ก claude / codex / git และการ login
agent-duo bridge …        MCP server ของ agent แต่ละตัว (agent เรียกเอง ไม่ต้องรันมือ)
agent-duo hook …          ด่านอนุมัติของ Claude Code (agent เรียกเอง)
```

ต้องมีสำหรับ real agents (อย่างน้อยหนึ่งอย่าง):
- **Claude**: Claude Code ติดตั้งแล้วและ login (`claude`) หรือตั้ง `ANTHROPIC_API_KEY`
- **Codex**: `npm i -g @openai/codex` แล้ว `codex login` หรือตั้ง `OPENAI_API_KEY` (agent-duo หา `codex.exe` ตัวจริงที่อยู่หลัง `codex.cmd` ให้เอง)
- **git**: ถ้าโฟลเดอร์เป็น git repo

## ตั้งค่า session (หน้าแรก)

### ใครทำอะไร

ทุก AI ที่ติ๊กเข้าทีมมีช่องให้เขียนว่าทำอะไร และมีปุ่มลัด build / tests / review / docs / research
- lead ต้องแบ่งงานตามนี้อย่างเคร่งครัด ช่องว่างหมายถึงให้ lead เลือกเอง
- ถ้าในช่องมี `review only`, `do not edit`, `read-only`, `ห้ามแก้` หรือ `อ่านอย่างเดียว` ระบบ**บังคับ**ให้ AI ตัวนั้นอ่านได้อย่างเดียว

### โหมด

เลือกตอนเริ่ม สลับได้ด้วย `/mode` หรือกดที่ chip "mode"

| โหมด | Claude | Codex |
|---|---|---|
| **auto** | แก้ไฟล์และรันคำสั่งเอง | แก้ไฟล์และรันคำสั่งเอง |
| **accept edits** | แก้ไฟล์เอง ทุกคำสั่ง shell รอคุณกด allow | อ่านได้อย่างเดียว (`codex exec` ถามกลางคำสั่งไม่ได้) |
| **plan** | อ่านได้อย่างเดียว | อ่านได้อย่างเดียว |

plan: lead เสนอแผนละเอียดให้อนุมัติก่อน พออนุมัติแล้วสลับไปโหมดที่เลือกไว้

### ไฟล์สำคัญ (Protected files)

ใส่ pattern ทีละบรรทัด เช่น `.env*`, `db/migrations/**` (pattern ที่ไม่มี `/` ใช้ได้ทุกโฟลเดอร์)
- Claude ถูกดักก่อนแก้ไฟล์ที่ตรง pattern และรอคุณอนุมัติ
- ตอน merge ไฟล์สำคัญที่ถูกแก้แต่ยังไม่อนุมัติจะถูกถามทีละไฟล์ ตอบ no แล้วไฟล์ถูกคืนค่า
- ถ้ามีไฟล์สำคัญ จะไม่ให้แก้โฟลเดอร์โดยตรง (ใช้ branch หรือสำเนาแทน)

### AI เจ้าอื่น (Other AI)

DeepSeek, Kimi, GLM, OpenRouter, Ollama (รูปแบบ Anthropic รันบน Claude Code) หรือ LM Studio (OpenAI Responses รันบน Codex) กด **test connection** ได้ key เก็บที่ `~/.agent-duo/providers.json` (หรือ `DUO_HOME`) เท่านั้น หน้าเว็บเห็นแค่ `sk-…abcd` และ key ไม่ถูกเขียนลง log

## ใช้งานในหน้า session

| พิมพ์ | ผล |
|---|---|
| ข้อความธรรมดา | ส่งให้ lead |
| `@codex#1 ข้อความ` | ส่งให้ agent ตัวเดียว (หรือกดที่ agent ในผัง) |
| `y` / `n` / `e <สิ่งที่อยากแก้>` | ตอบการขออนุมัติล่าสุด |
| `/orc codex` · `/orc claude` | สลับ lead |
| `/mode auto` · `/mode edits` · `/mode plan` | สลับโหมด |
| `/kill codex#1.b` | หยุด agent ตัวนั้นพร้อม sub (ฆ่าทั้ง process tree ด้วย job object) |
| `/budget 10` · `/merge` · `/stop` · `/help` | |

**การแจ้งเตือน** (เมื่อหน้าต่างไม่ได้อยู่ข้างหน้า): มีเรื่องรออนุมัติ, lead ทำเสร็จ, session หยุดเพราะเกินงบหรือผิดพลาด, agent ล้มเหลว และ agent ส่งข้อความถึงคุณ
- แอป desktop: แจ้งเตือนของระบบ พร้อมกระพริบ taskbar / เด้งไอคอนใน Dock กลับมาที่หน้าต่างแล้วจะเปิด session ที่แจ้งให้เอง (แจ้งได้ทุก session ที่กำลังทำงาน) macOS จะถามขออนุญาตครั้งแรก
- เวอร์ชันเบราว์เซอร์: ใช้ Web Notification ของเบราว์เซอร์ ถามอนุญาตตอนกดเริ่ม session หรือส่งข้อความครั้งแรก แจ้งเฉพาะ session ที่เปิดแท็บค้างไว้
- ปิดแอป/แท็บแล้วจะไม่มีการแจ้งเตือน (agent หยุดไปพร้อมกัน)

แท็บ **timeline**: แท่งทึบคือช่วงทำงาน ลายเฉียงคือช่วงรอ เส้นประคือจุดที่แตก sub เส้นม่วงคือข้อความระหว่าง agent ขีดแดงคือ step ที่ล้มเหลว

## ทำงานอย่างไร

- agent ทุกตัวได้ MCP server ชื่อ `duo` (`agent-duo bridge …` ในไฟล์ exe เดียวกัน) ซึ่งส่งทุก tool call กลับมาที่ session ทาง HTTP session จึงตัดสินทุกอย่าง (ใครแจกงานได้, ความลึก, จำนวน agent, งบ)
  - lead ได้: `propose_split`, `assign`, `spawn_sub`, `wait`, `message`, `inbox`, `status`
  - ทีมและ sub ได้: `spawn_sub` (ถ้ายังไม่ถึงความลึกสูงสุด), `wait`, `message`, `inbox`, `status`
- **Claude**: `claude -p --output-format stream-json` + `--mcp-config` (bridge) + `--allowedTools` + `--permission-mode dontAsk`, turn ถัดไปใช้ `--resume` ทุก Edit/Write/Bash/PowerShell ผ่าน PreToolUse hook (`agent-duo hook`) ที่ตรวจโหมดและไฟล์สำคัญ
- **Codex**: `codex exec --json` ตั้ง MCP ผ่าน `-c mcp_servers.duo.*` sandbox `workspace-write` หรือ `read-only` ตามสิทธิ์ turn ถัดไปใช้ `codex exec resume`
- พื้นที่ทำงาน:
  - **git repo**: worktree ที่ `<repo>/.duo/worktrees/<session>` บน branch `duo/<session>` merge เมื่อคุณอนุมัติ
  - **โฟลเดอร์ทั่วไป**: สำเนาที่ `runs/<session>/workspace` (`node_modules`/`.venv` ใช้ junction ส่วน `dist`, `build`, `.next`, `target` ไม่คัดลอก) คัดลอกกลับเมื่ออนุมัติ ไฟล์ที่คุณแก้ซ้ำระหว่างนั้นถูกข้ามและแจ้งเป็น conflict
  - **แก้โฟลเดอร์โดยตรง**: ไม่มีขั้นอนุมัติตอนท้าย
- เกินงบเมื่อไหร่ ทุก agent หยุดทันที
- ทุก event อยู่ที่ `%APPDATA%\agent-duo\runs\<session>\events.jsonl` เปิดดูย้อนหลังได้ (รูปแบบเดียวกับ v0.3 ทุกตัวอักษร)
- ฟังเฉพาะ `127.0.0.1` ไม่มีระบบ login API รับเฉพาะคำขอจากหน้าเว็บของตัวเอง (ตรวจ Host, Origin และ JSON)

## ตั้งค่าผ่าน env

| ตัวแปร | ใช้ทำอะไร |
|---|---|
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` | ถ้าไม่ได้ login Claude Code / Codex |
| `CLAUDE_MODEL` / `CODEX_MODEL` | เปลี่ยน model |
| `CODEX_USD_PER_MTOK_IN` / `_OUT` | ราคา Codex ต่อ 1M token |
| `RUNS_DIR` | ที่เก็บ session (ค่าเริ่มต้น `%APPDATA%\agent-duo\runs`) |
| `DUO_HOME` | ที่เก็บ `providers.json` (ค่าเริ่มต้น `~/.agent-duo`) |
| `DUO_CLAUDE` / `DUO_CODEX` | path ของ `claude.exe` / `codex.exe` ถ้าหาเองไม่เจอ |

skills และ subagents ของ agent-duo: โฟลเดอร์ `shared/` ข้าง ๆ ที่รัน หรือ `%APPDATA%\agent-duo\shared` (Claude โหลดเป็น plugin, Codex ได้สำเนาใน `.agents/skills` ระหว่างทำงาน)

## build เอง

ต้องมี Rust, Node (เฉพาะตอน build หน้าเว็บ) และบน Windows แบบ GNU ต้องมี mingw-w64 (`dlltool`, `windres`) ใน PATH เช่น `winget install BrechtSanders.WinLibs.POSIX.UCRT`

```bash
cd web && npm ci && npm run build && cd ..   # หน้าเว็บ → web/dist (ถูกฝังใน exe)
cargo test --workspace                        # ทดสอบทั้งหมด และสร้าง type ของ TS ใน web/src/bindings
cargo build --release -p agent-duo            # → target/release/agent-duo.exe
```

แอป desktop (ต้องมี Node สำหรับ Tauri CLI):

```bash
cd desktop && npm ci
npx tauri dev                 # เปิดแอปจากโค้ด
npx tauri build               # installer ของเครื่องนี้ → target/release/bundle/
```

- installer ของ macOS ต้อง build บน Mac (หรือใช้ GitHub Actions) Windows build ข้าม Mac ไม่ได้
- build บน Windows แบบ GNU ต้องวาง `WebView2Loader.dll` (อยู่ใน `target/release/`) ไว้ข้าง exe ส่วน build แบบ MSVC (ใน GitHub Actions) ลิงก์ไว้ในตัวแล้ว

พัฒนาหน้าเว็บ: `cargo run -p agent-duo -- --no-open` แล้ว `cd web && npm run dev` (http://localhost:5173 proxy `/api` ไปที่พอร์ต 3000)

push tag `v*` ขึ้น GitHub แล้ว `.github/workflows/release.yml` จะทดสอบ แล้ว build บน Windows และ macOS: installer ของแอป desktop (`.exe`/`.msi`, `.dmg` แบบ universal) และ `agent-duo` command-line ทั้งสองระบบ ใส่ไว้ใน draft release (ถ้าตั้ง secrets `APPLE_*` ไว้ แอป macOS จะถูก sign และ notarize)

## โครงสร้าง

```
core/src/types.rs       event และ config (serde + ts-rs → web/src/bindings)
core/src/session.rs     หัวใจ: agent tree, เครื่องมือ duo, การอนุมัติ, โหมด, ไฟล์สำคัญ, คำสั่ง, ขีดจำกัด, merge
core/src/prompts.rs     prompt ของ lead / สมาชิก / การส่งต่อ lead
core/src/runners/       mock · claude · codex · proc (หา CLI, อ่าน JSONL, job object)
core/src/git.rs         worktree, diff, commit, merge, คืนค่าไฟล์ (git command line)
core/src/copy.rs        โหมดสำเนาสำหรับโฟลเดอร์ที่ไม่ใช่ git
core/src/providers.rs   AI เจ้าอื่น
core/tests/             flow.rs (mock ครบลูป, /kill, งบ, จำนวน agent) · workspace.rs (copy, git) · fixtures (events.jsonl จาก v0.3)
server/src/api.rs       axum: sessions, SSE, input, approvals, tool, gate, providers + หน้าเว็บที่ฝังไว้ (rust-embed)
server/src/bridge.rs    `bridge` (MCP ผ่าน rmcp) และ `hook`
server/src/main.rs      เปิด server / `doctor`
web/                    React + Vite (หน้า / และ /s/:id)
desktop/                แอป Tauri: เปิด engine ในโปรเซสแล้วแสดงหน้าเว็บในหน้าต่าง (src/main.rs, tauri.conf.json, icons/)
```

## สิ่งที่ยังต่างจากเวอร์ชัน TypeScript

- **แอป macOS ยังไม่ได้รันจริง**: โค้ดของ engine และ server ผ่านการ type-check สำหรับ macOS แล้ว แต่ตัวแอปต้อง build บน Mac หรือ GitHub Actions และยังไม่ได้เปิดบน Mac จริง
- **ยังไม่ได้ทดสอบกับ agent จริง** (ขั้นที่ 6): โค้ด Claude/Codex runner, bridge และ hook เขียนครบและ `doctor` ผ่าน แต่ยังไม่ได้รันลูปจริงกับโฟลเดอร์ ควรลองกับ repo เล็กก่อน
- Claude เรียกผ่าน CLI แทน Agent SDK: ด่านอนุมัติเป็น hook แบบ command (`agent-duo hook`) แทน callback ในโปรเซส ถ้าเรียก session ไม่ได้ hook จะ**ปฏิเสธ** (fail closed) และ timeout ของ MCP tool ตั้งผ่าน `MCP_TOOL_TIMEOUT`
- Codex ใช้ `--json` (SDK เดิมใช้ `--experimental-json`)
- session อยู่ที่ `%APPDATA%\agent-duo\runs` แทน `./runs` session เก่าไม่ถูกย้ายให้ (คัดลอกโฟลเดอร์เข้าไป หรือตั้ง `RUNS_DIR`)
- API ทุกตัวที่หน้าเว็บใช้ตรวจ Host/Origin/JSON (เดิมตรวจเฉพาะ providers) และ bridge เรียกกลับที่ `127.0.0.1:<port>` เสมอ
- `maxDepth` / `maxAgents` ที่เป็นทศนิยมถูกปัดลง (เดิมเก็บเป็นทศนิยม)
- โหมดสำเนาใช้ hash 64-bit ในหน่วยความจำแทน sha1 (ผลเหมือนกัน)
- โค้ด Next.js เดิม (`app/`, `components/`, `lib/`, `mcp/`) ยังอยู่ใน repo เพื่อใช้เทียบ ลบได้เมื่อมั่นใจเวอร์ชัน Rust แล้ว
