# โหมดการทำงาน, ไฟล์สำคัญ และสมาชิกทีมจากเจ้าอื่น — Design

วันที่: 2026-11-11 · สถานะ: ผู้ใช้อนุมัติให้เริ่มทำ

## ทำไม

ผู้ใช้อยากใช้ agent-duo กับงานที่ "ไม่อยากให้ agent ทำเองทั้งหมด" หรือมีจุดที่ sensitive และอยากเพิ่ม AI เจ้าอื่น (เช่น DeepSeek, local AI) เข้ามาเป็นสมาชิกทีม โดย key ต้องอยู่บนเครื่องเท่านั้น

## เป้าหมาย

1. **โหมดของ session**: `auto`, `accept edits`, `plan` เลือกได้ตอนเริ่ม และสลับกลางทางด้วย `/mode`
2. **ไฟล์สำคัญ**: ผู้ใช้กำหนด pattern เองได้ ถ้าไม่กำหนดก็แก้ได้ทุกไฟล์ ถ้า agent จะแก้ไฟล์ที่ตรง pattern ต้อง**ถามผู้ใช้ก่อน**
3. **สมาชิกจากเจ้าอื่น**: เพิ่ม provider (DeepSeek, Kimi, GLM, OpenRouter, Ollama, LM Studio, หรือกรอกเอง) แล้วใช้เป็นสมาชิกทีมคู่กับ Claude/Codex ได้ เป็น lead ได้ด้วย
4. **key อยู่บนเครื่อง**: เก็บในไฟล์นอกโฟลเดอร์โปรเจกต์ ไม่ส่ง key เต็มไป browser ไม่เขียนลง `runs/` หรือ log

## ไม่ทำ (non-goals)

- ไม่มีค่าตั้งต้นของไฟล์สำคัญ (ผู้ใช้สั่ง: ไม่กำหนด = แก้ได้หมด)
- ไม่เขียน agent loop เอง: provider ทุกตัววิ่งบน Claude Code หรือ Codex ที่มีอยู่
- ไม่เข้ารหัส key ในไฟล์ (ป้องกันด้วยตำแหน่งไฟล์ + สิทธิ์ไฟล์ เหมือน `~/.codex/auth.json`)
- ไม่มีระบบ login (เหมือนเดิม: ใช้บนเครื่องตัวเองเท่านั้น)

## สิ่งที่ถือว่าล้มเหลว (anti-goals)

- key หลุดไปที่ browser, `events.jsonl`, log, หรือ commit เข้า repo
- ไฟล์สำคัญถูกแก้และ merge เข้า repo โดยผู้ใช้ไม่ได้ตอบ "yes"
- โหมด plan แล้ว agent แก้ไฟล์ก่อนผู้ใช้อนุมัติแผน

## ข้อจำกัดทางเทคนิค (ตรวจแล้ว)

- **Claude Agent SDK** มี `canUseTool` callback: ดักการใช้เครื่องมือก่อนทำงาน และส่งไปถามผู้ใช้ได้จริง
- **Codex SDK** ไม่มี approval callback และ `codex exec` ตอบ approval ไม่ได้ ("command execution approval is not supported in exec mode") → Codex ถามผู้ใช้กลางคำสั่งไม่ได้
- **Codex 0.167** รองรับ provider แบบ OpenAI **Responses API** เท่านั้น (`wire_api = "chat"` ถูกถอดแล้ว) → DeepSeek (ไม่มี Responses API) ต้องวิ่งบน Claude Code ผ่าน endpoint แบบ Anthropic
- Claude Code ต่อ endpoint แบบ Anthropic ได้ผ่าน env `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN` + `ANTHROPIC_MODEL`
- Codex ต่อ provider ได้ผ่าน config `model_providers.<id>` + `model_provider`

## การออกแบบ

### 1. โหมด

`SessionConfig.permission: "auto" | "acceptEdits" | "plan"` และ `afterPlan: "auto" | "acceptEdits"`

| โหมด | Claude engine | Codex engine |
|---|---|---|
| auto | แก้ไฟล์ + รันคำสั่งเอง (เหมือนเดิม) | sandbox `workspace-write`, approval `never` (เหมือนเดิม) |
| accept edits | แก้ไฟล์เอง, **คำสั่ง shell (Bash/PowerShell) ทุกครั้งถามผู้ใช้** ผ่านกล่องอนุมัติ | `read-only` (ดูหมายเหตุตอนทำ) |
| plan | ทุก agent อ่านอย่างเดียว | ทุก agent `read-only` |

- **plan**: lead สำรวจแบบอ่านอย่างเดียว แล้วต้องเรียก `propose_split` พร้อมแผนละเอียดใน `summary` ขั้นอนุมัติ split ถูกบังคับเปิดในโหมดนี้ เมื่อผู้ใช้ตอบ yes โหมดเปลี่ยนเป็น `afterPlan` อัตโนมัติ แล้วทีมเริ่มทำงาน
- `/mode auto|edits|plan` สลับได้ทุกเมื่อ Claude agent ใช้โหมดใหม่ทันที (callback อ่านโหมดปัจจุบันทุกครั้ง) ส่วน Codex ใช้โหมดใหม่ตั้งแต่ turn ถัดไป
- มี event ใหม่ `mode_changed` และหัวหน้าจอ session แสดงโหมดปัจจุบัน

### 2. ไฟล์สำคัญ

`SessionConfig.protect: string[]` คือ glob ที่อิงจาก root ของโปรเจกต์ pattern ที่ไม่มี `/` จะเทียบกับชื่อไฟล์ทุกระดับด้วย (`.env*` ตรงกับ `config/.env.local`) ใช้ `path.matchesGlob` ของ Node

- **ระหว่างทำงาน (Claude)**: `canUseTool` ดัก `Edit`/`Write`/`MultiEdit`/`NotebookEdit` ที่ path ตรง pattern แล้วถามผู้ใช้ (approval ชนิดใหม่ `file`) ถ้า yes ให้จำ path นั้นไว้ใน `approvedFiles`
- **ตอน merge (ทุก engine และเป็นด่านสุดท้าย)**: ก่อน merge/คัดลอกกลับ ไฟล์ที่เปลี่ยนและตรง pattern แต่ยังไม่อยู่ใน `approvedFiles` จะถูกถามทีละไฟล์ ถ้า no ไฟล์นั้นถูกคืนค่าใน workdir (git: `git checkout`/ลบไฟล์ใหม่, copy: คัดลอกต้นฉบับกลับ/ลบไฟล์ใหม่) แล้วค่อย merge ที่เหลือ ด่านนี้ครอบคลุม Codex และการแก้ผ่าน Bash
- ถ้ามี pattern และเลือก "edit this folder directly" ระบบจะบังคับใช้ git worktree หรือสำเนาแทน และแจ้ง log (การตัดสินใจของ Claude เพราะโหมดตรงไม่มีขั้น merge ให้กัน)
- ฟอร์มจำ pattern ไว้แยกตาม path โปรเจกต์ใน `localStorage` (การตัดสินใจของ Claude: ไม่ใช่ความลับ)

### 3. Provider (สมาชิกจากเจ้าอื่น)

**ที่เก็บ**: `~/.agent-duo/providers.json` (เปลี่ยนที่ได้ด้วย env `DUO_HOME`) เขียนด้วยสิทธิ์ `0600`

```ts
type Provider = {
  id: string;            // ใช้เป็น kind ของ agent เช่น "deepseek" → deepseek#2
  label: string;
  engine: "claude" | "codex";   // claude = API แบบ Anthropic, codex = OpenAI Responses API
  baseUrl: string;
  apiKey: string;        // อยู่บน server เท่านั้น
  model: string;
  usdPerMTokIn?: number; usdPerMTokOut?: number;
};
```

**API** (`app/api/providers`): GET คืน list ที่ key ถูก mask (`sk-…abcd`), PUT upsert (ช่อง key ว่าง = ใช้ค่าเดิม), DELETE, POST `/test` ยิงคำขอเล็กที่สุดไปที่ `baseUrl` ของ provider นั้นเท่านั้น

**Preset** ในฟอร์ม (กรอก base URL/engine ให้, ชื่อ model เป็นตัวอย่างให้แก้): DeepSeek (claude, `https://api.deepseek.com/anthropic`), Kimi (claude, `https://api.moonshot.ai/anthropic`), GLM (claude, `https://api.z.ai/api/anthropic`), OpenRouter (claude, `https://openrouter.ai/api`), Ollama (claude, `http://localhost:11434`), LM Studio (codex, `http://localhost:1234/v1`) และ "กรอกเอง" ค่า preset ยังไม่ได้ทดสอบกับทุกเจ้า ให้ใช้ปุ่มทดสอบเป็นตัวยืนยัน

**การรัน**
- `Kind` กลายเป็น `string`: `"claude"`, `"codex"` หรือ provider id session หา engine ของแต่ละ kind ตอนสร้าง runner
- Claude engine + provider: `env = {...process.env, ANTHROPIC_BASE_URL, ANTHROPIC_AUTH_TOKEN: key, ANTHROPIC_API_KEY: "", ANTHROPIC_MODEL, ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU}_MODEL, ANTHROPIC_SMALL_FAST_MODEL = model, CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1}`
- Codex engine + provider: `config.model_providers.<id> = {name, base_url, env_key: "DUO_PROVIDER_KEY", wire_api: "responses"}`, `model_provider = id`, model และ `env.DUO_PROVIDER_KEY = key`
- ค่าใช้จ่ายของ provider คำนวณจาก token × ราคาที่กรอก (ไม่กรอก = $0 แต่ยังแสดง token) ไม่ใช้ `total_cost_usd` ของ Claude Code เพราะคิดเป็นราคา Anthropic
- อ่าน key จากไฟล์ตอนเริ่มแต่ละ turn ไม่ใส่ใน `SessionConfig` (เพราะ config ถูกเขียนลง `events.jsonl`)
- ทุกจุดที่เคยเช็ก `kind === "codex"` (copy skills, codexAgents) เปลี่ยนเป็นเช็ก engine
- MCP bridge: `agent` ใน `propose_split`/`assign` เปลี่ยนจาก enum เป็น string ส่วน session ตรวจว่าอยู่ในทีมหรือไม่
- prompt ของ lead ระบุสมาชิกทุกตัวพร้อม label และ engine
- UI: สีของ provider สร้างจาก id (hash ไปยัง palette), ฟอร์มเริ่มมีส่วน "Other AI" สำหรับเพิ่ม/แก้/ทดสอบ/ลบ provider, provider ที่เพิ่มแล้วเลือกเป็น lead หรือติ๊กเข้าทีมได้

## หมายเหตุตอนทำ

- Codex 0.167 ไม่รองรับ `approval_policy = "untrusted"` แล้ว (turn ล้มทันที) และใน sandbox `workspace-write` คำสั่งไม่ต้องขออนุมัติเลย จึงกั้นคำสั่งของ Codex ไม่ได้ ใช้ทางสำรอง: Codex อ่านอย่างเดียวในโหมด accept edits และ prompt ของ lead บอกให้ส่งงานแก้ไฟล์ให้ agent ฝั่ง Claude
- ใช้ PreToolUse hook แทน `canUseTool` เพราะ hook ทำงานแม้ allow rule ใน `settings.json` ของผู้ใช้จะปล่อยผ่าน
- บน Windows Claude Code ใช้เครื่องมือ `PowerShell` แทน `Bash` จึงกั้นทั้งสองตัว
- ทดสอบจริงแล้ว: Claude ถามก่อนเขียน `.env`, ถามก่อนรันคำสั่งและเคารพคำตอบ no, การเขียนไฟล์ทั่วไปไม่ถาม, และด่านตอน merge คืนค่า `.env.local` ที่สร้างผ่าน shell ใน git repo ส่วน provider จริง (DeepSeek, Ollama, …) ยังไม่ได้ทดสอบเพราะไม่มี key/server

## สิ่งที่ต้องทดสอบตอนทำ

- Codex + `approvalPolicy: "untrusted"` ใน exec: ต้องปฏิเสธคำสั่งแบบสะอาด ไม่ทำให้ทั้ง turn ล้ม ถ้าล้ม ให้ใช้ทางสำรอง: Codex ในโหมด accept edits ใช้ sandbox `read-only` สำหรับคำสั่ง และแจ้งผู้ใช้
- mock mode ต้องยังทำงานครบ
- `npm run typecheck`
- เช็กว่า GET `/api/providers` ไม่มี key เต็ม และ key ไม่อยู่ใน `events.jsonl`

## ปล่อยให้ผู้ทำตัดสินใจ

- หน้าตา UI ในรายละเอียด
- ข้อความ approval และ prompt
