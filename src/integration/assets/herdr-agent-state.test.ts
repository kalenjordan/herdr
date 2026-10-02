import { afterEach, expect, test } from "bun:test";
import { rm, mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { createServer, type Server } from "node:net";
import { spawn, execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

const originalEnvironment = {
  HERDR_ENV: process.env.HERDR_ENV,
  HERDR_PANE_ID: process.env.HERDR_PANE_ID,
  HERDR_SOCKET_PATH: process.env.HERDR_SOCKET_PATH,
};

let server: Server | undefined;
let socketPath: string | undefined;
let importCounter = 0;

afterEach(async () => {
  await new Promise<void>((resolve, reject) => {
    if (!server) {
      resolve();
      return;
    }
    server.close((error) => (error ? reject(error) : resolve()));
  });
  server = undefined;

  if (socketPath) {
    await rm(socketPath, { force: true });
    socketPath = undefined;
  }

  for (const [name, value] of Object.entries(originalEnvironment)) {
    if (value === undefined) {
      delete process.env[name];
    } else {
      process.env[name] = value;
    }
  }
});

const integrations = [
  { name: "Pi", modulePath: "./pi/herdr-agent-state.ts" },
  { name: "Oh My Pi", modulePath: "./omp/herdr-agent-state.ts" },
] as const;

function importFresh(modulePath: string) {
  importCounter += 1;
  return import(`${modulePath}?test=${importCounter}`);
}

type Handler = (event: unknown, context: unknown) => unknown;

function createExtensionHarness() {
  const handlers = new Map<string, Handler>();
  return {
    handlers,
    pi: {
      on(event: string, handler: Handler) {
        handlers.set(event, handler);
      },
      events: {
        on() {
          return () => {};
        },
      },
    },
  };
}

function configureIntegrationEnvironment(recordingSocketPath: string) {
  process.env.HERDR_ENV = "1";
  process.env.HERDR_SOCKET_PATH = recordingSocketPath;
  process.env.HERDR_PANE_ID = "test:p1";
}

async function startRecordingServer(name: string): Promise<unknown[]> {
  const recordingSocketPath = join(tmpdir(), `herdr-${name}-${process.pid}.sock`);
  socketPath = recordingSocketPath;
  await rm(recordingSocketPath, { force: true });

  const requests: unknown[] = [];
  const recordingServer = createServer((socket) => {
    let input = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline === -1) {
        return;
      }
      requests.push(JSON.parse(input.slice(0, newline)));
      socket.end("{}\n");
    });
  });
  server = recordingServer;
  await new Promise<void>((resolve, reject) => {
    recordingServer.once("error", reject);
    recordingServer.listen(recordingSocketPath, resolve);
  });
  configureIntegrationEnvironment(recordingSocketPath);
  return requests;
}

for (const integration of integrations) {
  test(`${integration.name} reload preserves working state when the agent is active`, async () => {
    const requests = await startRecordingServer(
      integration.name.toLowerCase().replaceAll(" ", "-"),
    );
    const { handlers, pi } = createExtensionHarness();

    const { default: install } = await importFresh(integration.modulePath);
    install(pi);

    const sessionStart = handlers.get("session_start");
    expect(sessionStart).toBeDefined();
    await sessionStart?.(
      { reason: "reload" },
      {
        hasUI: true,
        isIdle: () => false,
        sessionManager: {
          getSessionFile: () => undefined,
          getSessionId: () => undefined,
        },
      },
    );

    const reportedState = () => {
      for (const request of requests) {
        if (!isRecord(request) || request.method !== "pane.report_agent") {
          continue;
        }
        const params = request.params;
        if (isRecord(params) && typeof params.state === "string") {
          return params.state;
        }
      }
      return undefined;
    };

    const deadline = Date.now() + 1_000;
    while (Date.now() < deadline && reportedState() === undefined) {
      await Bun.sleep(5);
    }

    expect(reportedState()).toBe("working");
  });
}

test("Codex session hook reports to the sole live pane in its directory", async () => {
  const recordingSocketPath = join(tmpdir(), `herdr-codex-live-pane-${process.pid}.sock`);
  socketPath = recordingSocketPath;
  await rm(recordingSocketPath, { force: true });
  const reportedPanes: string[] = [];
  let claimedByAnotherSession = false;
  server = createServer((socket) => {
    let input = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline < 0) return;
      const request = JSON.parse(input.slice(0, newline));
      if (request.method === "pane.list") {
        socket.end(JSON.stringify({ result: { panes: [
          { pane_id: "old:p1", cwd: "/elsewhere", agent: "codex" },
          { pane_id: "live:p1", cwd: "/project", agent: "codex",
            ...(claimedByAnotherSession ? { agent_session: { kind: "id", value: "another-session" } } : {}) },
        ] } }) + "\n");
      } else if (request.method === "pane.process_info") {
        socket.end('{"result":{"process_info":{"foreground_processes":[]}}}\n');
      } else {
        reportedPanes.push(request.params.pane_id);
        socket.end('{"result":{"type":"ok"}}\n');
      }
    });
  });
  await new Promise<void>((resolve) => server?.listen(recordingSocketPath, resolve));
  const child = spawn("sh", [join(import.meta.dir, "codex/herdr-agent-state.sh"), "session"], {
    env: { ...process.env, HERDR_ENV: "1", HERDR_PANE_ID: "old:p1", HERDR_SOCKET_PATH: recordingSocketPath },
    stdio: ["pipe", "pipe", "pipe"],
  });
  child.stdin.end(JSON.stringify({ hook_event_name: "SessionStart", session_id: "codex-session", cwd: "/project" }));
  const exitCode = await new Promise<number | null>((resolve) => child.on("close", resolve));
  expect(exitCode).toBe(0);
  expect(reportedPanes).toEqual(["live:p1"]);

  claimedByAnotherSession = true;
  const otherChild = spawn("sh", [join(import.meta.dir, "codex/herdr-agent-state.sh"), "session"], {
    env: { ...process.env, HERDR_ENV: "1", HERDR_PANE_ID: "old:p1", HERDR_SOCKET_PATH: recordingSocketPath },
    stdio: ["pipe", "pipe", "pipe"],
  });
  otherChild.stdin.end(JSON.stringify({ hook_event_name: "SessionStart", session_id: "codex-session", cwd: "/project" }));
  expect(await new Promise<number | null>((resolve) => otherChild.on("close", resolve))).toBe(0);
  expect(reportedPanes).toEqual(["live:p1"]);
});

test("Codex hook proves pane ownership through ancestry despite worktree cwd and delayed session", async () => {
  socketPath = join(tmpdir(), `herdr-codex-ancestry-${process.pid}.sock`);
  await rm(socketPath, { force: true });
  const reports: string[] = [];
  server = createServer(socket => {
    let input = "";
    socket.on("data", chunk => {
      input += chunk;
      if (!input.includes("\n")) return;
      const request = JSON.parse(input.slice(0, input.indexOf("\n")));
      if (request.method === "pane.list") {
        socket.end(JSON.stringify({ result: { panes: [
          { pane_id: "owner", cwd: "/main", agent: "codex" },
          { pane_id: "other", cwd: "/main", agent: "codex" },
        ] } }) + "\n");
      } else if (request.method === "pane.process_info") {
        socket.end(JSON.stringify({ result: { process_info: { foreground_processes: [
          { name: "codex", pid: request.params.pane_id === "owner" ? process.pid : 99999999 },
        ] } } }) + "\n");
      } else {
        reports.push(request.params.pane_id);
        socket.end('{"result":{"type":"ok"}}\n');
      }
    });
  });
  await new Promise<void>(resolve => server?.listen(socketPath, resolve));
  const child = spawn("sh", [join(import.meta.dir, "codex/herdr-agent-state.sh"), "session"], {
    env: { ...process.env, HERDR_ENV: "1", HERDR_PANE_ID: "other", HERDR_SOCKET_PATH: socketPath },
    stdio: ["pipe", "pipe", "pipe"],
  });
  child.stdin.end(JSON.stringify({ hook_event_name: "SessionStart", source: "resume",
    session_id: "delayed-session", cwd: "/worktree" }));
  expect(await new Promise(resolve => child.on("close", resolve))).toBe(0);
  expect(reports).toEqual(["owner"]);
});

test("shared-server hook matches a delayed worktree session and rejects ambiguous launches", async () => {
  const fixture = await mkdtemp(join(tmpdir(), "herdr-hook-worktree-"));
  try {
    const main = join(fixture, "main"), worktree = join(fixture, "worktree"), bin = join(fixture, "bin");
    execFileSync("git", ["init", main]);
    execFileSync("git", ["-C", main, "-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "--allow-empty", "-m", "init"]);
    execFileSync("git", ["-C", main, "worktree", "add", "--detach", worktree]);
    await mkdir(bin);
    await writeFile(join(bin, "ps"), '#!/bin/sh\ncase "$*" in *ppid=*) echo 1;; *) echo "Thu Oct  1 19:02:28 2026";; esac\n', { mode: 0o755 });
    socketPath = join(fixture, "hook.sock");
    let ambiguous = false;
    const reports: string[] = [];
    server = createServer(socket => {
      let input = "";
      socket.on("data", chunk => {
        input += chunk;
        if (!input.includes("\n")) return;
        const request = JSON.parse(input.slice(0, input.indexOf("\n")));
        if (request.method === "pane.list") socket.end(JSON.stringify({ result: { panes:
          ["owner", ...(ambiguous ? ["other"] : [])].map(pane_id => ({ pane_id, cwd: main, agent: "codex" })) } }) + "\n");
        else if (request.method === "pane.process_info") socket.end(JSON.stringify({ result: { process_info: {
          foreground_processes: [{ name: "codex", pid: 99999999 }] } } }) + "\n");
        else { reports.push(request.params.pane_id); socket.end('{"result":{"type":"ok"}}\n'); }
      });
    });
    await new Promise<void>(resolve => server?.listen(socketPath, resolve));
    const run = async () => {
      const child = spawn("sh", [join(import.meta.dir, "codex/herdr-agent-state.sh"), "session"], {
        env: { ...process.env, PATH: bin + ":" + process.env.PATH, HERDR_ENV: "1", HERDR_PANE_ID: "stale", HERDR_SOCKET_PATH: socketPath },
        stdio: ["pipe", "pipe", "pipe"],
      });
      child.stdin.end(JSON.stringify({ hook_event_name: "SessionStart", source: "startup", session_id: "delayed",
        cwd: worktree, transcript_path: join(fixture, "rollout-2026-10-01T19-02-34-delayed.jsonl") }));
      expect(await new Promise(resolve => child.on("close", resolve))).toBe(0);
    };
    await run();
    expect(reports).toEqual(["owner"]);
    ambiguous = true;
    await run();
    expect(reports).toEqual(["owner"]);
  } finally { await rm(fixture, { recursive: true, force: true }); }
});

test("Codex clear claims the focused numeric tab despite a stale inherited pane", async () => {
  const recordingSocketPath = join(tmpdir(), `herdr-codex-clear-${process.pid}.sock`);
  socketPath = recordingSocketPath;
  await rm(recordingSocketPath, { force: true });
  const reportedPanes: string[] = [];
  let focused = true;
  server = createServer((socket) => {
    let input = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline < 0) return;
      const request = JSON.parse(input.slice(0, newline));
      if (request.method === "pane.list") {
        socket.end(JSON.stringify({ result: { panes: [
          { pane_id: "live:p1", tab_id: "live:t1", cwd: "/project", agent: "codex", focused,
            agent_session: { kind: "id", value: "old-session" } },
          { pane_id: "other:p1", tab_id: "other:t1", cwd: "/project", agent: "codex", focused: false,
            agent_session: { kind: "id", value: "other-session" } },
        ] } }) + "\n");
      } else if (request.method === "tab.get") {
        socket.end('{"result":{"tab":{"label":"5"}}}\n');
      } else if (request.method === "pane.process_info") {
        socket.end('{"result":{"process_info":{"foreground_processes":[]}}}\n');
      } else {
        reportedPanes.push(request.params.pane_id);
        socket.end('{"result":{"type":"ok"}}\n');
      }
    });
  });
  await new Promise<void>((resolve) => server?.listen(recordingSocketPath, resolve));
  const runHook = async () => {
    const child = spawn("sh", [join(import.meta.dir, "codex/herdr-agent-state.sh"), "session"], {
      env: { ...process.env, HERDR_ENV: "1", HERDR_PANE_ID: "other:p1", HERDR_SOCKET_PATH: recordingSocketPath },
      stdio: ["pipe", "pipe", "pipe"],
    });
    child.stdin.end(JSON.stringify({ hook_event_name: "SessionStart", source: "clear",
      session_id: "new-session", cwd: "/project" }));
    expect(await new Promise<number | null>((resolve) => child.on("close", resolve))).toBe(0);
  };
  await runHook();
  expect(reportedPanes).toEqual(["live:p1"]);
  focused = false;
  await runHook();
  expect(reportedPanes).toEqual(["live:p1"]);
});

test("Pi reports the session replacement source", async () => {
  const requests = await startRecordingServer("pi-session-source");
  const { handlers, pi } = createExtensionHarness();

  const { default: install } = await importFresh("./pi/herdr-agent-state.ts");
  install(pi);

  const sessionStart = handlers.get("session_start");
  expect(sessionStart).toBeDefined();
  await sessionStart?.(
    { reason: "new" },
    {
      hasUI: true,
      isIdle: () => true,
      sessionManager: {
        getSessionFile: () => "/tmp/pi-new.jsonl",
        getSessionId: () => "pi-new",
      },
    },
  );

  const reportedSession = () =>
    requests.find((request) => isRecord(request) && request.method === "pane.report_agent_session");
  const deadline = Date.now() + 1_000;
  while (Date.now() < deadline && reportedSession() === undefined) {
    await Bun.sleep(5);
  }

  const request = reportedSession();
  expect(request).toBeDefined();
  expect(isRecord(request) && isRecord(request.params) ? request.params.session_start_source : null)
    .toBe("new");
});

test("Pi waits for a replacement session report before publishing state", async () => {
  const recordingSocketPath = join(tmpdir(), `herdr-pi-session-order-${process.pid}.sock`);
  socketPath = recordingSocketPath;
  await rm(recordingSocketPath, { force: true });

  const requests: unknown[] = [];
  let acknowledgeSessionReport: (() => void) | undefined;
  const recordingServer = createServer((socket) => {
    let input = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline === -1) {
        return;
      }
      const request = JSON.parse(input.slice(0, newline));
      requests.push(request);
      if (isRecord(request) && request.method === "pane.report_agent_session") {
        acknowledgeSessionReport = () => socket.end("{}\n");
        return;
      }
      socket.end("{}\n");
    });
  });
  server = recordingServer;
  await new Promise<void>((resolve, reject) => {
    recordingServer.once("error", reject);
    recordingServer.listen(recordingSocketPath, resolve);
  });

  configureIntegrationEnvironment(recordingSocketPath);
  const { handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./pi/herdr-agent-state.ts");
  install(pi);

  const sessionStart = handlers.get("session_start");
  expect(sessionStart).toBeDefined();
  const sessionStartResult = sessionStart?.(
    { reason: "new" },
    {
      hasUI: true,
      isIdle: () => false,
      sessionManager: {
        getSessionFile: () => "/tmp/pi-new.jsonl",
        getSessionId: () => "pi-new",
      },
    },
  );

  const deadline = Date.now() + 1_000;
  while (Date.now() < deadline && acknowledgeSessionReport === undefined) {
    await Bun.sleep(5);
  }
  expect(acknowledgeSessionReport).toBeDefined();
  expect(
    requests.some((request) => isRecord(request) && request.method === "pane.report_agent"),
  ).toBe(false);

  acknowledgeSessionReport?.();
  await sessionStartResult;

  const stateDeadline = Date.now() + 1_000;
  while (
    Date.now() < stateDeadline &&
    !requests.some((request) => isRecord(request) && request.method === "pane.report_agent")
  ) {
    await Bun.sleep(5);
  }
  expect(requests.map((request) => (isRecord(request) ? request.method : undefined))).toEqual([
    "pane.report_agent_session",
    "pane.report_agent",
  ]);
});

test("Pi retries working state after an unanswered socket attempt", async () => {
  const recordingSocketPath = join(tmpdir(), `herdr-pi-retry-${process.pid}.sock`);
  socketPath = recordingSocketPath;
  await rm(recordingSocketPath, { force: true });

  let connectionCount = 0;
  const attemptedRequests: unknown[] = [];
  const deliveredRequests: unknown[] = [];
  const recordingServer = createServer((socket) => {
    connectionCount += 1;
    const connectionNumber = connectionCount;
    let input = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline === -1) {
        return;
      }
      const request = JSON.parse(input.slice(0, newline));
      attemptedRequests.push(request);
      if (connectionNumber === 1) {
        return;
      }
      deliveredRequests.push(request);
      socket.end("{}\n");
    });
  });
  server = recordingServer;
  await new Promise<void>((resolve, reject) => {
    recordingServer.once("error", reject);
    recordingServer.listen(recordingSocketPath, resolve);
  });

  configureIntegrationEnvironment(recordingSocketPath);
  const { handlers, pi } = createExtensionHarness();

  const { default: install } = await importFresh("./pi/herdr-agent-state.ts");
  install(pi);

  const sessionStart = handlers.get("session_start");
  expect(sessionStart).toBeDefined();
  await sessionStart?.(
    { reason: "startup" },
    {
      hasUI: true,
      isIdle: () => false,
      sessionManager: {
        getSessionFile: () => undefined,
        getSessionId: () => undefined,
      },
    },
  );

  const reportedWorking = () =>
    deliveredRequests.some((request) => {
      if (!isRecord(request) || request.method !== "pane.report_agent") {
        return false;
      }
      const params = request.params;
      return isRecord(params) && params.state === "working";
    });

  const deadline = Date.now() + 2_500;
  while (Date.now() < deadline && !reportedWorking()) {
    await Bun.sleep(5);
  }

  expect(connectionCount).toBeGreaterThanOrEqual(2);
  expect(attemptedRequests.length).toBeGreaterThanOrEqual(2);
  expect(attemptedRequests[1]).toEqual(attemptedRequests[0]);
  expect(reportedWorking()).toBe(true);
});

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
