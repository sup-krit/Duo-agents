# Agent Duo v0.3

คุยกับทีม AI แบบ CLI: เลือก **lead (orchestrator)** จะเป็น Claude, Codex หรือ AI เจ้าอื่นก็ได้ lead แบ่งงานให้ทีมตามที่คุณกำหนดว่า AI ตัวไหนทำอะไร agent ทุกตัว **แตก sub-agent ของตัวเองได้** คุณเห็นผังว่าใครแตกใครออกมา และดู **timeline** ได้ว่าใครทำงานช่วงไหน รอใคร ส่งข้อความหากันเมื่อไหร่

```
You ─▶ lead (Claude / Codex / DeepSeek …) ── propose_split ──▶ you approve
                    │
                    ├─ codex#1     build ──spawn_sub──▶ codex#1.a, codex#1.b
                    ├─ deepseek#2  tests
                    └─ claude#3    review   (after #1, #2)
```

## มีอะไรใหม่ใน v0.3

- **ใครทำอะไร**: เขียนเองได้ว่า AI แต่ละตัวทำอะไร มีปุ่มลัด build / tests / review / docs / research และ lead ต้องแบ่งงานตามนี้
- **โหมด**: auto · accept edits · plan
- **ไฟล์สำคัญ**: กำหนด pattern ของไฟล์ที่ต้องได้รับอนุมัติก่อนแก้
- **AI เจ้าอื่น**: DeepSeek, Kimi, GLM, OpenRouter, Ollama, LM Studio หรือกรอกเอง เข้าทีมหรือเป็น lead ได้ key เก็บบนเครื่องเท่านั้น
- **skills และ subagents**: ใช้ skill/agent ของ repo เป้าหมาย, `~/.claude`, `~/.codex` และโฟลเดอร์ `shared/` ของ agent-duo ได้ทั้ง Claude และ Codex
- แก้บั๊ก: บน Windows, Claude Code รันคำสั่งผ่านเครื่องมือ `PowerShell` ซึ่งก่อนหน้านี้ถูกปฏิเสธเสมอ

## เริ่มใช้

```bash
npm install
npm run dev        # http://localhost:3000
```

- **mock (no keys)**: agent จำลองเดินตาม script ดู flow ทั้งหมดได้ทันที (เสนอการแบ่งงาน, แตก sub, ส่งข้อความ, merge) ไม่แก้ไฟล์ใด ๆ
- **real agents**: ใส่ path ของโฟลเดอร์โปรเจกต์ (เป็น git หรือไม่ก็ได้) ต้องมีอย่างน้อยหนึ่งอย่าง
  - Claude: login Claude Code ไว้บนเครื่อง หรือตั้ง `ANTHROPIC_API_KEY`
  - Codex: `codex login` หรือตั้ง `OPENAI_API_KEY` (CLI ติดมากับ `@openai/codex-sdk`)
  - AI เจ้าอื่น: เพิ่มในส่วน "Other AI" ของฟอร์ม (ดูด้านล่าง)
  - เปิดเว็บผ่าน origin ที่ agent เรียกกลับได้ เช่น `http://localhost:3000`

## ตั้งค่า session (หน้าแรก)

### ใครทำอะไร

ในส่วน "Team it can hand work to" ทุก AI ที่ติ๊กไว้มีช่องให้เขียนเองว่าทำอะไร และมีปุ่มลัด

| ปุ่ม | ข้อความที่เติมให้ (แก้ต่อได้) |
|---|---|
| build | write and change the code |
| tests | write and run the tests |
| review | review only: read the changes and report problems, do not edit files |
| docs | write and update documentation |
| research | read the code and find answers, do not edit files |

- lead ต้องแบ่งงานตามนี้อย่างเคร่งครัด ถ้าไม่มี AI ตัวไหนเหมาะกับงานชิ้นหนึ่ง lead ต้องบอกในแผนแทนการฝืนแจก ช่องว่างหมายถึงให้ lead เลือกเอง
- agent แต่ละตัวได้รับคำสั่งนี้ด้วย และถือเหนือกว่างานที่ lead แจกถ้าขัดกัน
- ถ้าในช่องมี `review only`, `do not edit`, `read-only`, `ห้ามแก้` หรือ `อ่านอย่างเดียว` ระบบจะ**บังคับ**ให้ AI ตัวนั้นอ่านได้อย่างเดียว
- นอกจากนั้นระบบตรวจไม่ได้ว่างานตรงบทบาทไหม ด่านจริงคือตอนคุณอนุมัติแผนการแบ่งงาน
- ค่าเริ่มต้นคือ Codex = build, Claude = review ฟอร์มจำค่าไว้ให้ครั้งหน้า

### โหมด

เลือกตอนเริ่ม สลับได้ด้วย `/mode` หรือกดที่ chip "mode" ด้านบนหน้า session

| โหมด | Claude (และ AI ที่รันบน Claude Code) | Codex (และ AI ที่รันบน Codex) |
|---|---|---|
| **auto** | แก้ไฟล์และรันคำสั่งเอง | แก้ไฟล์และรันคำสั่งเอง |
| **accept edits** | แก้ไฟล์เอง แต่ทุกคำสั่ง shell ต้องรอคุณกด allow | อ่านได้อย่างเดียว* |
| **plan** | อ่านได้อย่างเดียว | อ่านได้อย่างเดียว |

- **plan**: lead สำรวจแล้วเสนอแผนละเอียดให้คุณอนุมัติก่อน ขั้นนี้บังคับแม้ปิด "starting the split" ไว้ พออนุมัติแล้วจะสลับไปโหมดที่เลือกไว้ (auto หรือ accept edits) และทีมเริ่มทำงาน
- \* Codex หยุดถามกลางคำสั่งไม่ได้ (`codex exec` ไม่มีขั้นอนุมัติ และ Codex 0.167 เลิกรองรับ `untrusted` แล้ว) จึงอ่านได้อย่างเดียวในโหมดนี้ lead จะส่งงานแก้ไฟล์ให้ฝั่ง Claude แทน

### ไฟล์สำคัญ (Protected files)

ใส่ pattern ทีละบรรทัด เช่น `.env*`, `db/migrations/**`, `src/payment/**` pattern ที่ไม่มี `/` ใช้ได้ทุกโฟลเดอร์ ถ้าไม่ใส่ก็แก้ได้ทุกไฟล์ ฟอร์มจำค่าไว้แยกตามโฟลเดอร์

- Claude ถูกดักก่อนแก้ไฟล์ที่ตรง pattern และต้องรอคุณอนุมัติ
- **ด่านสุดท้ายตอน merge**: ไฟล์สำคัญที่ถูกแก้แต่คุณยังไม่อนุมัติ (เช่น Codex แก้ หรือแก้ผ่านคำสั่ง shell) จะถูกถามทีละไฟล์ ถ้าตอบ no ไฟล์นั้นถูกคืนค่า ส่วนไฟล์อื่น merge ตามปกติ
- ถ้ามีไฟล์สำคัญ จะเลือก "edit this folder directly" ไม่ได้ (เพราะไม่มีขั้น merge ให้กัน) ระบบจะใช้ branch หรือสำเนาแทน

### AI เจ้าอื่น (Other AI)

อยู่ในฟอร์มเมื่อเลือก real agents กด `+ DeepSeek`, `+ Ollama (local)` ฯลฯ หรือ `+ other…` แล้วกรอก key, model และกด **test connection** AI ที่เพิ่มแล้วติ๊กเข้าทีมหรือเลือกเป็น lead ได้ ทุกเจ้ารันบน Claude Code หรือ Codex จึงได้เครื่องมือ, sub-agent, skills, โหมด และการป้องกันไฟล์สำคัญครบ

| รูปแบบ API | รันบน | ตัวอย่าง |
|---|---|---|
| Anthropic | Claude Code (`ANTHROPIC_BASE_URL`) | DeepSeek `https://api.deepseek.com/anthropic`, Kimi, GLM, OpenRouter, Ollama |
| OpenAI **Responses** | Codex (`model_providers`) | LM Studio `http://localhost:1234/v1` |

- Codex 0.167 ไม่รองรับ Chat Completions แล้ว เจ้าที่มีแค่ Chat Completions จึงต้องใช้ endpoint แบบ Anthropic ถ้ามี
- ชื่อ model ใน preset เป็นแค่ตัวอย่าง ตรวจกับรายชื่อ model ของเจ้านั้นก่อน
- ค่าใช้จ่ายคิดจากราคาต่อ 1M token ที่กรอกไว้ ถ้าไม่กรอกจะนับเป็น $0 (ยังเห็นจำนวน token)

**key อยู่บนเครื่องเท่านั้น**
- เก็บที่ `~/.agent-duo/providers.json` (เปลี่ยนที่ได้ด้วย env `DUO_HOME`) นอกโฟลเดอร์โปรเจกต์
- หน้าเว็บเห็นแค่ `sk-…abcd` key ไม่ถูกเขียนลง `runs/`, `events.jsonl` หรือ log
- ถ้าเปลี่ยน base URL ต้องใส่ key ใหม่ key เดิมจะไม่ถูกส่งไปที่ URL ใหม่
- API ที่จัดการ key รับเฉพาะคำขอ JSON จาก origin เดียวกัน

## ใช้งานในหน้า session

| พิมพ์ | ผล |
|---|---|
| ข้อความธรรมดา | ส่งให้ lead |
| `@codex#1 ข้อความ` | ส่งให้ agent ตัวเดียว (หรือกดที่ agent ในผัง แล้วพิมพ์ได้เลย) |
| `y` / `n` / `e <สิ่งที่อยากแก้>` | ตอบการขออนุมัติล่าสุด (หรือกดปุ่มในกล่อง) |
| `/orc codex` · `/orc claude` · `/orc deepseek` | สลับ lead (ทำได้ตอน lead ไม่ได้อยู่กลาง turn) |
| `/mode auto` · `/mode edits` · `/mode plan` | สลับโหมด |
| `/kill codex#1.b` | หยุด agent ตัวนั้นพร้อม sub ของมัน |
| `/budget 10` · `/merge` · `/stop` · `/help` | |

กล่องขออนุมัติมี 5 แบบ: แผนการแบ่งงาน (split), sub-agent, merge, คำสั่ง shell (accept edits) และไฟล์สำคัญ

แท็บ **timeline** แสดงทุก agent ตามเวลา: แท่งทึบคือช่วงทำงาน ลายเฉียงคือช่วงรอ เส้นประคือจุดที่แตก sub เส้นม่วงคือข้อความระหว่าง agent และขีดแดงคือ step ที่ล้มเหลว การ์ดด้านบนบอกจำนวน agent ที่ทำงานพร้อมกันโดยเฉลี่ย เวลาที่ถูก block และ chain ที่ช้าที่สุด

## Skills และ subagents

- **Claude**: ใช้ `Skill` และ `Agent` ได้ จาก `.claude/skills`, `.claude/agents` ของ repo เป้าหมาย, `~/.claude/` และ `shared/skills`, `shared/agents` ของ agent-duo (เรียกเป็น `shared:<ชื่อ>`) subagent ได้สิทธิ์ไม่เกิน agent ที่เรียกมัน
- **Codex**: อ่าน `.agents/skills` ของ repo เป้าหมายและ `~/.codex/skills` เอง
  - `shared/skills` ถูกคัดลอกเข้า `<workdir>/.agents/skills/` ตอน Codex เริ่ม turn (ถ้า repo มี skill ชื่อเดียวกัน ใช้ของ repo) แล้วลบออกก่อน diff/merge และตอนจบ session จึงไม่หลุดเข้า repo คุณ
  - `shared/agents/*.md` (รูปแบบเดียวกับ Claude: frontmatter `name`, `description` + เนื้อหาเป็น prompt) ถูกแปลงเป็น role ของ Codex เรียกผ่าน `spawn_agent` ด้วย `agent_type` ได้ ช่อง `tools`/`model` ไม่ใช้ และ role ได้ sandbox เดียวกับ agent ที่เรียก
- repo เป้าหมายที่เป็น git: worktree มีแค่ไฟล์ที่ commit แล้ว skill ที่ยังไม่ commit จะไม่เห็น

## ทำงานอย่างไร

- agent ทุกตัวได้ MCP server ชื่อ `duo` (`mcp/duo-bridge.mjs`) ซึ่งส่งทุก tool call กลับมาที่ session ทาง HTTP session จึงเป็นผู้ตัดสินทุกอย่าง (ใครแจกงานได้, ความลึก, จำนวน agent, งบประมาณ) และผังถูกต้องเสมอ
  - lead ได้: `propose_split`, `assign`, `spawn_sub`, `wait`, `message`, `inbox`, `status`
  - ทีมและ sub ได้: `spawn_sub` (ถ้ายังไม่ถึงความลึกสูงสุด), `wait`, `message`, `inbox`, `status`
- สิทธิ์: lead, agent ที่ role มีคำว่า review และ AI ที่คุณเขียนว่าห้ามแก้ **อ่านได้อย่างเดียว** ตัวอื่นแก้ไฟล์และรันคำสั่งได้ตามโหมด
  - Claude: ทุก Edit/Write/Bash/PowerShell (รวมของ subagent) ผ่าน PreToolUse hook ที่ตรวจโหมดและไฟล์สำคัญ hook ทำงานแม้ `settings.json` ของคุณจะมี allow rule
  - Codex: sandbox `workspace-write` หรือ `read-only` ตามโหมด
- พื้นที่ทำงาน (agent ทุกตัวใช้ร่วมกัน):
  - **โฟลเดอร์เป็น git repo**: worktree ที่ `<repo>/.duo/worktrees/<session>` บน branch `duo/<session>` แล้ว merge เมื่อคุณอนุมัติ
  - **โฟลเดอร์ทั่วไป**: **สำเนา** ที่ `runs/<session>/workspace` (`node_modules`/`.venv` ใช้ลิงก์ ส่วน `dist`, `build`, `.next` ไม่คัดลอก) แล้วคัดลอกไฟล์ที่เปลี่ยนกลับเมื่อคุณอนุมัติ ถ้าคุณแก้ไฟล์เดียวกันระหว่างนั้น ไฟล์นั้นจะถูกข้ามและแจ้งเป็น conflict
  - **แก้โฟลเดอร์โดยตรง** (ติ๊ก "edit this folder directly"): ไม่มีขั้นอนุมัติตอนท้าย และย้อนกลับไม่ได้ถ้าโฟลเดอร์ไม่ได้อยู่ใน git
- ถ้าปิด "Ask me before merging" ระบบ merge หรือคัดลอกกลับให้อัตโนมัติเมื่อ lead จบ turn (ไฟล์สำคัญยังถูกถามเสมอ)
- เกินงบเมื่อไหร่ ทุก agent หยุดทันที
- ทุก event ถูกเก็บใน `runs/<session>/events.jsonl` เปิดดูย้อนหลังได้

## ตั้งค่าผ่าน env (`.env.local`)

| ตัวแปร | ใช้ทำอะไร |
|---|---|
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` | ถ้าไม่ได้ login Claude Code / Codex |
| `CLAUDE_MODEL` / `CODEX_MODEL` | เปลี่ยน model ของ Claude / Codex |
| `CODEX_USD_PER_MTOK_IN` / `_OUT` | ราคา Codex ต่อ 1M token สำหรับตัวนับค่าใช้จ่าย |
| `RUNS_DIR` | ที่เก็บ log ของ session (ค่าเริ่มต้น `./runs`) |
| `DUO_HOME` | ที่เก็บ `providers.json` (ค่าเริ่มต้น `~/.agent-duo`) |

## โครงสร้าง

```
lib/session.ts          หัวใจ: agent tree, เครื่องมือ duo, การอนุมัติ, โหมด, ไฟล์สำคัญ, คำสั่ง, ขีดจำกัด, merge
lib/prompts.ts          prompt ของ lead / สมาชิก / การส่งต่อ lead (แก้ตรงนี้เพื่อเปลี่ยนพฤติกรรม)
lib/runners/            claude.ts · codex.ts · mock.ts  (แปลงผลของแต่ละ SDK เป็น event, permission gate)
lib/providers.ts        AI เจ้าอื่น: ที่เก็บ key, ตรวจค่า, ทดสอบการเชื่อมต่อ
lib/shared.ts           ส่ง shared/skills และ shared/agents ให้ Claude และ Codex
lib/git.ts              worktree, diff, commit, merge, คืนค่าไฟล์
lib/workspace-copy.ts   โหมดสำเนาสำหรับโฟลเดอร์ที่ไม่ใช่ git
mcp/duo-bridge.mjs      MCP server (stdio) ที่ทุก agent ใช้
app/api/sessions/...    สร้าง session, SSE events, input, approvals, tool (ให้ bridge เรียก)
app/api/providers/...   เพิ่ม/แก้/ลบ/ทดสอบ AI เจ้าอื่น (ไม่ส่ง key เต็มกลับ)
components/duo/         start-form · providers · session-view · chat · tree · timeline · reduce
shared/                 skills/ และ agents/ ที่ทุก agent ใช้ร่วมกัน
docs/superpowers/specs/ เอกสารออกแบบ
```

## วิดีโอ demo

`video/` คือโปรเจกต์ Remotion ของวิดีโอ demo 90 วินาที (ข้อความไทย ไม่มีเสียง)

```bash
cd video && npm install
npm run studio                                   # ดูและแก้ใน browser
npm run render                                   # → video/out/agent-duo-v0.3.mp4
node scripts/capture.mjs http://localhost:3125   # ถ่ายภาพหน้าจอใหม่จาก server ที่ใช้ข้อมูล demo
```

## ข้อควรรู้

- ทดสอบแล้ว: mock mode ครบทุก flow และ Claude ของจริงในเรื่อง ไฟล์สำคัญ, การขออนุมัติคำสั่ง และด่านตอน merge ส่วน Codex ของจริงกับโหมดใหม่ และ AI เจ้าอื่นของจริง ยังไม่ได้ทดสอบ ควรลองกับ repo เล็กก่อน
- agent หลายตัวแก้ worktree เดียวกันพร้อมกัน ถ้าแบ่งงานให้แตะไฟล์เดียวกันอาจชนกันได้ ให้ lead แบ่งตามไฟล์ (prompt บอกไว้แล้ว)
- session อยู่ในหน่วยความจำของ server ถ้า restart ระหว่างทำงาน session จะหยุด (ประวัติยังดูได้)
- ใช้บนเครื่องตัวเองเท่านั้น ไม่มีระบบ login อย่าเปิดให้เข้าถึงจากอินเทอร์เน็ต
- `components/ui/number-ticker.tsx` มาจาก affectUI (MIT)
