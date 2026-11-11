import type { Runner, TurnContext, TurnResult } from "./types";

// Scripted agents for trying the whole flow without API keys. They call the same duo tools
// as real agents (propose_split, spawn_sub, wait, message), so the tree, approvals, timeline
// and limits behave exactly as in real mode. They do not touch any files.

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    if (signal.aborted) return reject(new Error("stopped"));
    const t = setTimeout(resolve, ms);
    signal.addEventListener("abort", () => (clearTimeout(t), reject(new Error("stopped"))), { once: true });
  });

export function createMockRunner(): Runner {
  let turns = 0;

  return {
    async turn(ctx: TurnContext): Promise<TurnResult> {
      turns++;
      const { agent, signal, emit, callTool } = ctx;
      let n = 0;
      const say = (text: string) => emit({ type: "say", agent: agent.id, text });
      const step = async (verb: string, target: string, ms: number, fail?: string) => {
        const callId = `${agent.id}-${turns}-${n++}`;
        emit({ type: "tool", agent: agent.id, callId, verb, target, status: "running" });
        await sleep(ms, signal);
        emit({ type: "tool", agent: agent.id, callId, verb, target, status: fail ? "error" : "ok", detail: fail });
      };
      const waitAll = async (ids?: string[]) => {
        let all = "";
        for (;;) {
          const r = await callTool("wait", ids ? { ids } : {});
          all += r.text + "\n";
          if (r.isError || !/still running/i.test(r.text)) return all;
        }
      };
      const done = (text: string, tokens: number): TurnResult => ({ text, tokens, costUsd: tokens * (agent.kind === "claude" ? 0.000009 : 0.000005) });

      const role = agent.depth === 0 ? "lead" : agent.depth >= 2 ? "sub" : agent.role;

      if (role === "lead" && (turns > 1 || ctx.prompt.includes("taking over as LEAD"))) {
        await sleep(800, signal);
        await callTool("status", {});
        const last = ctx.prompt.split("\n").pop() ?? "";
        say(`Noted: "${last.slice(0, 80)}". Nothing else to hand out right now. (mock lead)`);
        return done("Acknowledged.", 900);
      }

      if (role === "lead") {
        say("Looking at the code first.");
        await step("read", "src/api/users.ts", 700);
        await step("grep", '"createUser|updateUser"  2 hits', 600);
        await step("read", "package.json", 400);
        say("Three parts. Build and tests can run side by side; review waits for both.");
        let r = await callTool("propose_split", {
          summary: "Build and tests in parallel, then an independent review.",
          items: [
            { agent: "codex", role: "build", task: "Cache layer in src/lib/cache.ts, wire it into getUsers()" },
            { agent: "codex", role: "tests", task: "tests/cache.test.ts: hit, miss, expiry, invalidation" },
            { agent: "claude", role: "review", task: "Review the diff once build and tests pass", after: [1, 2] },
          ],
        });
        if (/^changes requested/i.test(r.text)) {
          say("Understood, folding tests into the build part.");
          r = await callTool("propose_split", {
            summary: "One builder that also writes the tests, then a review.",
            items: [
              { agent: "codex", role: "build", task: "Cache layer + wire into getUsers() + tests/cache.test.ts" },
              { agent: "claude", role: "review", task: "Review the diff once the build passes", after: [1] },
            ],
          });
        }
        if (r.isError || /^rejected/i.test(r.text)) {
          say("Okay, I won't start anything.");
          return done("Stopped before starting.", 3000);
        }
        await waitAll();
        say("All parts are done and the review approved the change. getUsers() is cached for 60s, invalidated on createUser/updateUser, with 11 passing tests.");
        return done("Cache added, tests pass, review approved.", 6100);
      }

      if (role === "build") {
        await step("read", "src/api/users.ts", 500);
        await step("grep", '"getUsers\\("  4 call sites', 500);
        say("Two independent pieces, splitting them.");
        const a = await callTool("spawn_sub", { role: "sub", task: "Write the TTL cache module" });
        const b = await callTool("spawn_sub", { role: "sub", task: "Invalidate on createUser / updateUser" });
        if (a.isError || b.isError) say(`Could not split (${a.isError ? a.text : b.text}); doing it myself.`);
        const got = await waitAll();
        if (/off-by-one|>=/.test(got)) {
          say("Right, fixing the expiry check in cache.ts.");
          await step("edit", "src/lib/cache.ts  +1 −1", 900);
        }
        await step("run", "npx tsc --noEmit", 1200);
        return done("Added src/lib/cache.ts (+38) and wired getUsers() through it; invalidation in createUser/updateUser (+9 −4). Fixed expiry (>=). Typecheck clean.", 18300);
      }

      if (role === "tests") {
        say("Checking how existing tests are set up first.");
        await callTool("spawn_sub", { role: "sub", task: "Find test helpers and DB mocks" });
        await waitAll();
        await step("add", "tests/cache.test.ts  +31", 1500);
        await step("run", "npm test", 1800, "1 failed · expires after ttl");
        const status = (await callTool("status", {})).text;
        const build = status.match(/^(\S+) · codex · build/m)?.[1];
        if (build) {
          await callTool("message", { to: build, text: "Expiry check looks off by one: `now > exp` should be `now >= exp`." });
          await waitAll([build]);
        }
        await step("run", "npm test", 1600);
        return done("tests/cache.test.ts covers hit, miss, expiry and invalidation. npm test: 11 passed.", 14200);
      }

      if (role === "review") {
        await step("run", "git diff --stat", 600);
        await step("read", "src/lib/cache.ts", 700);
        await step("read", "tests/cache.test.ts", 600);
        say("VERDICT: APPROVE. Invalidation covers both write paths and the tests exercise hit, miss, expiry and invalidation.");
        return done("APPROVE. No blocking issues.", 9400);
      }

      // sub-agents
      const t = agent.task.toLowerCase();
      if (t.includes("helper")) {
        await step("glob", "tests/**/*.ts", 700);
        await step("read", "tests/helpers/db.ts", 800);
        return done("Use createTestDb() from tests/helpers/db.ts and vi.useFakeTimers() for expiry.", 3100);
      }
      if (t.includes("invalidate")) {
        await sleep(2500, signal); // starts after the cache module exists
        await step("edit", "src/api/users.ts  +9 −4", 2600);
        await step("run", "npx tsc --noEmit", 1400);
        return done("createUser/updateUser now clear the users cache.", 4200);
      }
      await step("add", "src/lib/cache.ts  +38", 2200);
      await step("run", "npx tsc --noEmit", 1300);
      return done("TTL cache module in src/lib/cache.ts (Map + expiry).", 5200);
    },
  };
}
