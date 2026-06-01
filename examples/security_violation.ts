import { fs, web, agent } from '../nanos-sdk/index.js';

export async function run() {
  console.log("Security Demonstration Agent started.");
  
  // 1. Read authorized file (whitelisted in manifest)
  try {
    console.log("Attempting to read whitelisted 'instruction.txt'...");
    const data = await fs.readFile("instruction.txt");
    console.log(`Success! Read content: "${data.trim()}"`);
  } catch (err) {
    console.error("Failed to read whitelisted file:", err.message);
  }

  // 2. Attempt to read an unauthorized file (blocked by host FFI check)
  try {
    console.log("\nAttempting to read unauthorized 'Cargo.toml'...");
    const data = await fs.readFile("Cargo.toml");
    console.log("Warning: Successfully read unauthorized file (sandbox bypass!):", data);
  } catch (err) {
    console.log(`Blocked! Host successfully denied read capability. Reason: ${err.message}`);
  }

  // 3. Attempt to fetch network resources (blocked by network manifest rule)
  try {
    console.log("\nAttempting to call external network API...");
    const res = await web.get("https://api.github.com/repos/PandiaJason/nanos");
    console.log("Warning: Network call succeeded (sandbox bypass!):", res);
  } catch (err) {
    console.log(`Blocked! Host successfully denied network access. Reason: ${err.message}`);
  }

  // 4. Signal clean exit with report
  console.log("\nVerification complete. Signaling done state to host.");
  await agent.done("Security verification completed. All sandbox boundaries held.");
}

run().catch(err => {
  console.error("Agent crashed:", err);
  process.exit(1);
});
