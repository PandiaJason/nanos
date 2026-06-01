import fs from 'fs';

function sendSyscall(method, params) {
  const req = JSON.stringify({ jsonrpc: "2.0", method, params }) + "\n";
  // Write to stdout (fd 1) synchronously
  fs.writeSync(1, req);

  // Read response from stdin (fd 0) synchronously
  const buf = Buffer.alloc(1);
  let line = "";
  while (true) {
    const bytesRead = fs.readSync(0, buf, 0, 1, null);
    if (bytesRead === 0) break;
    const char = buf.toString('utf8');
    if (char === "\n") break;
    line += char;
  }

  const resp = JSON.parse(line);
  if (resp.error) {
    throw new Error(resp.error.message);
  }
  return resp.result;
}

export const fs_api = {
  readFile: (path) => sendSyscall("fs_read", [path]),
  writeFile: (path, content) => sendSyscall("fs_write", [path, content]),
};

// Map default export alias for convenience
export { fs_api as fs };

export const llm = {
  infer: (prompt) => sendSyscall("llm_infer", [prompt]),
};

export const agent = {
  getGoal: () => sendSyscall("get_manifest_goal", []),
  done: (summary) => sendSyscall("done", [summary]),
};

export const web = {
  get: (url) => sendSyscall("web_get", [url]),
};

export const mcp = {
  call: (server, tool, args) => sendSyscall("mcp_call", [server, tool, args]),
};

export const fleet = {
  send: (target, msg) => sendSyscall("agent_send", [target, msg]),
  recv: () => sendSyscall("agent_recv", []),
};

