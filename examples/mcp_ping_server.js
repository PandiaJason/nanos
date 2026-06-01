const readline = require('readline');

const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout,
  terminal: false
});

rl.on('line', (line) => {
  try {
    const request = JSON.parse(line);
    if (request.method === 'tools/call') {
      const response = {
        jsonrpc: "2.0",
        id: request.id,
        result: {
          content: [
            {
              type: "text",
              text: `Hello from MCP Ping Server! Arguments received: ${JSON.stringify(request.params.arguments)}`
            }
          ]
        }
      };
      console.log(JSON.stringify(response));
    } else {
      // Return empty/generic response for initialization or list_tools if called
      const response = {
        jsonrpc: "2.0",
        id: request.id,
        result: {}
      };
      console.log(JSON.stringify(response));
    }
  } catch (err) {
    console.error(err);
  }
});
