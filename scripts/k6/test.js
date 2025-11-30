import ws from "k6/ws";
import { check } from "k6";
import { Trend, Counter } from "k6/metrics";
import { sleep } from "k6";

// Metrics
const rttTrend = new Trend("rtt_ms");
const serverToClientTrend = new Trend("server_to_client_ms");
const errors = new Counter("errors");
const sent = new Counter("messages_sent");

let WS_URL = __ENV.WS_URL;

// messages per second per VU
const MSG_RATE = parseFloat(__ENV.MSG_RATE || "1");

// k6 main entry - each VU runs this function
export default function () {
  const intervalMs = Math.max(1, Math.floor(1000 / MSG_RATE));
  let seq = 0;

  const res = ws.connect(WS_URL, {}, function (socket) {
    socket.on("open", function () {
      // sanity check
      // send an initial ping
      socket.send("hello-from-k6");
    });

    // send messages at configured rate (per VU)
    socket.setInterval(function () {
      const now = Date.now();
      const id = `${__VU}-${seq++}`;
      // we prefix with an ID marker so we can find the message when it is broadcast back
      const payload = `ID:${id}    |{"t":${now},"vu":${__VU},"seq":${seq}}`;
      try {
        socket.send(payload);
        sent.add(1);
      } catch (e) {
        errors.add(1);
      }
    }, intervalMs);

    socket.on("message", function (msg) {
      const text = msg.toString();
      // First try to parse outer JSON emitted by the server
      let outer = null;
      try {
        outer = JSON.parse(text);
      } catch (e) {
        outer = null;
      }

      let payloadText = text;
      if (outer && typeof outer.message === 'string') {
        payloadText = outer.message;
        // optional: measure server->client delay if server timestamp available
        if (outer.timestamp) {
          const serverToClientMs = Date.now() - outer.timestamp;
          // record server->client latency in its own metric
          serverToClientTrend.add(serverToClientMs);
        }
      }

      // look for messages that belong to this VU (contain our instanceId)
      const marker = `ID:${__VU}-`;
      const idx = payloadText.indexOf(marker);
      if (idx !== -1) {
        const pipe = payloadText.indexOf("|", idx);
        if (pipe !== -1 && pipe + 1 < payloadText.length) {
          const jsonStr = payloadText.substring(pipe + 1);
          try {
            const data = JSON.parse(jsonStr);
            if (data && data.t) {
              const rtt = Date.now() - data.t;
              rttTrend.add(rtt);
            }
          } catch (e) {
            // ignore parse errors
          }
        }
      }
    });

    socket.on("close", function () {
      // nothing
    });

    socket.on("error", function (e) {
      errors.add(1);
    });
  });

  check(res, { "connected": (r) => r && r.status === 101 });

  // keep the VU alive for the duration of the test (k6 controls global duration)
  // small sleep to yield
  sleep(1);
}

export function teardown(data) {
  // no-op
}

/*
Usage examples:

# Run 100 virtual users for 60s, each sending 1 msg/s
WS_URL=ws://localhost:6142/ws MSG_RATE=1 k6 run --vus 100 --duration 60s scripts/k6/ws_test.js

# Use wss and higher rate
WS_URL=wss://til-store-draws-charts.trycloudflare.com/ws MSG_RATE=2 k6 run --vus 200 --duration 60s scripts/k6/ws_test.js

Metrics produced (k6 summary):
- rtt_ms: Trend (you can inspect p(50), p(95), p(99) in k6 output)
- errors: number of socket/send errors
- messages_sent: total messages sent

Collect server-side CPU/memory while running (examples):
  pid=$(pgrep -f chat-server)
  top -p $pid
  ls /proc/$pid/fd | wc -l    # fd count

*/
