import { mcp, agent } from '../nanos-sdk/index.js';

export async function run() {
  console.log("TS MCP Caller Agent started.");
  
  try {
    console.log("Calling MCP 'ping' tool on 'ping-server' with argument 'hello_from_ts'...");
    const response = await mcp.call("ping-server", "ping", { input: "hello_from_ts" });
    console.log(`Success! Raw MCP response: "${response}"`);
  } catch (err) {
    console.error("Failed to invoke MCP server:", err.message);
  }

  console.log("Signaling done state to host.");
  await agent.done("MCP server caller execution completed successfully.");
}

run().catch(err => {
  console.error("Agent crashed:", err);
  process.exit(1);
});
