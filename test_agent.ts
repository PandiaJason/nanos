import { fs, llm, agent } from './nanos-sdk/index.js';

export async function run() {
  console.log("TS Agent started!");
  const goal = await agent.getGoal();
  console.log("TS Goal received:", goal);

  const inputData = await fs.readFile("instruction.txt");
  console.log("TS Read instruction.txt:", inputData);

  const response = await llm.infer(`Summarize code: ${inputData}`);
  console.log("TS LLM Inference result:", response);

  await fs.writeFile("secret.txt", response);
  console.log("TS Wrote secret.txt");

  await agent.done("TS FFI Loop completed successfully.");
}

run().catch(err => {
  console.error("TS Agent execution failed:", err);
  process.exit(1);
});
