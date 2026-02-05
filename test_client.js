const WebSocket = require('ws');

const ws = new WebSocket('ws://localhost:3000/ws');

let step = 0;

ws.on('open', function open() {
  console.log('Connected');
  
  // Test 1: Hello
  console.log('Sending Hello...');
  ws.send(JSON.stringify({ message: "Hello" }));
});

ws.on('message', function incoming(data) {
  try {
      const msg = JSON.parse(data);
      if (msg.type === 'text_delta') {
        process.stdout.write(msg.delta);
      } else if (msg.type === 'done') {
        console.log('\nDONE');
        // Test 2: Tool Call
        if (step === 0) {
            step = 1;
            console.log('\nSending Tool Request: List files in current directory');
            ws.send(JSON.stringify({ message: "List files in current directory" }));
        } else {
            process.exit(0);
        }
      }
  } catch (e) {
      console.log("Raw:", data.toString());
  }
});
