import http from "node:http";

const port = Number(process.env.PORT ?? 3000);
const daemonUrl = process.env.DAEMON_URL ?? "http://127.0.0.1:8080";
const apiKey = process.env.API_KEY;

function json(res, status, body) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

async function readJson(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  return JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
}

async function forward(path, body, method = "POST") {
  const response = await fetch(`${daemonUrl}${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: method === "GET" ? undefined : JSON.stringify(body)
  });
  const text = await response.text();
  return {
    status: response.status,
    body: text ? JSON.parse(text) : {}
  };
}

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === "GET" && req.url === "/health") {
      return json(res, 200, { ok: true });
    }

    if (apiKey && req.headers.authorization !== `Bearer ${apiKey}`) {
      return json(res, 401, { ok: false, error: "unauthorized" });
    }

    if (req.method === "GET" && req.url === "/handles") {
      const result = await forward("/handles", {}, "GET");
      return json(res, result.status, result.body);
    }

    if (req.method === "POST" && req.url === "/messages") {
      const body = await readJson(req);
      if (typeof body.to !== "string" || typeof body.text !== "string") {
        return json(res, 400, {
          ok: false,
          error: "`to` and `text` must be strings"
        });
      }

      const result = await forward("/send", {
        to: body.to,
        text: body.text
      });
      return json(res, result.status, result.body);
    }

    if (req.method === "POST" && req.url === "/availability") {
      const body = await readJson(req);
      if (typeof body.to !== "string") {
        return json(res, 400, {
          ok: false,
          error: "`to` must be a string"
        });
      }

      const result = await forward("/availability", {
        to: body.to
      });
      return json(res, result.status, result.body);
    }

    if (req.method === "POST" && req.url === "/facetime/calls") {
      const body = await readJson(req);
      if (typeof body.to !== "string") {
        return json(res, 400, {
          ok: false,
          error: "`to` must be a string"
        });
      }

      const result = await forward("/facetime/call", {
        to: body.to
      });
      return json(res, result.status, result.body);
    }

    if (req.method === "POST" && req.url === "/admin/provision") {
      const body = await readJson(req);
      if (
        typeof body.apple_id !== "string" ||
        typeof body.password !== "string"
      ) {
        return json(res, 400, {
          ok: false,
          error: "`apple_id` and `password` must be strings"
        });
      }

      const result = await forward("/provision", body);
      return json(res, result.status, result.body);
    }

    if (req.method === "POST" && req.url === "/admin/provision/complete") {
      const body = await readJson(req);
      if (typeof body.two_factor_code !== "string") {
        return json(res, 400, {
          ok: false,
          error: "`two_factor_code` must be a string"
        });
      }

      const result = await forward("/provision/complete", body);
      return json(res, result.status, result.body);
    }

    if (req.method === "POST" && req.url === "/admin/provision/sms") {
      const result = await forward("/provision/sms", {});
      return json(res, result.status, result.body);
    }

    if (req.method === "POST" && req.url === "/admin/cache/clear") {
      const result = await forward("/cache/clear", {});
      return json(res, result.status, result.body);
    }

    if (req.method === "POST" && req.url === "/admin/reregister") {
      const result = await forward("/reregister", {});
      return json(res, result.status, result.body);
    }

    return json(res, 404, { ok: false, error: "not_found" });
  } catch (error) {
    return json(res, 500, {
      ok: false,
      error: "api_error",
      message: error instanceof Error ? error.message : String(error)
    });
  }
});

server.listen(port, "::", () => {
  console.log(`api listening on [::]:${port}`);
});
