#!/usr/bin/env python3
"""Mock backend for the zniff demo video.

Serves a small "shop" API with realistic latency, sizes and content types so
that the traffic table looks like a real session: JSON, GraphQL, a JWT login,
protobuf, a PNG, a JS bundle, a 304, a 500 and a breakpoint target.

usage: mock_server.py <port> [png-path]
"""

import base64
import http.server
import json
import random
import socketserver
import sys
import time

PORT = int(sys.argv[1])
PNG = open(sys.argv[2], "rb").read() if len(sys.argv) > 2 else b""

random.seed(7)


def b64url(obj):
    raw = json.dumps(obj, separators=(",", ":")).encode()
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


JWT = ".".join([
    b64url({"alg": "HS256", "typ": "JWT"}),
    b64url({"sub": "usr_9482", "name": "Alice Chen", "role": "admin", "iat": 1794547200, "exp": 1794633600}),
    "kR3n0xGm7Yq2sB8vLzP4cW1hT6dJ9fA0uE5iN2oQ7x",
])

PRODUCTS = [
    {"id": "sku_1001", "name": "Mechanical Keyboard", "price": 129.0, "stock": 42, "tags": ["peripheral", "rgb"]},
    {"id": "sku_1002", "name": "4K Monitor 27\"", "price": 349.0, "stock": 7, "tags": ["display"]},
    {"id": "sku_1003", "name": "USB-C Dock", "price": 89.5, "stock": 120, "tags": ["hub", "thunderbolt"]},
    {"id": "sku_1004", "name": "Noise-cancelling Headphones", "price": 279.0, "stock": 0, "tags": ["audio"]},
]

GQL = {
    "data": {
        "viewer": {
            "id": "usr_9482",
            "name": "Alice Chen",
            "email": "alice@zniff.app",
            "plan": "PRO",
            "orders": {
                "totalCount": 3,
                "edges": [
                    {"node": {"id": "ord_88219", "status": "SHIPPED", "total": 478.0}},
                    {"node": {"id": "ord_88102", "status": "DELIVERED", "total": 89.5}},
                    {"node": {"id": "ord_87990", "status": "REFUNDED", "total": 129.0}},
                ],
            },
        }
    }
}

PROTO = (
    b"\x08\x96\x01"
    b"\x12\x05zniff"
    b"\x1a\x1atelemetry.v1.ClusterStatus"
    b"\x20\xea\x0f"
    b"\x2a\x0bap-east-1-b"
    b"\x30\x01"
)

JS = ("/*! shop-web v2.14.0 */\n" + "(()=>{\"use strict\";const e=document;" * 60 + "})();\n").encode()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def version_string(self):
        return "shop-api/2.14"

    def reply(self, status, body=b"", ctype="application/json; charset=utf-8", extra=None, delay=(0.02, 0.09)):
        time.sleep(random.uniform(*delay))
        if isinstance(body, (dict, list)):
            body = json.dumps(body, indent=2).encode()
        self.send_response(status)
        if body or status not in (204, 304):
            self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("X-Request-Id", "req_%06x" % random.randrange(1 << 24))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if body:
            self.wfile.write(body)

    def body(self):
        n = int(self.headers.get("Content-Length", 0) or 0)
        return self.rfile.read(n) if n else b""

    def do_GET(self):
        p = self.path
        if p.startswith("/api/v1/products"):
            self.reply(200, {"page": 1, "pageSize": 4, "total": 128, "items": PRODUCTS}, delay=(0.08, 0.16))
        elif p == "/api/v1/cart":
            self.reply(200, {"items": [{"sku": "sku_1001", "qty": 1}, {"sku": "sku_1003", "qty": 2}], "subtotal": 308.0})
        elif p == "/api/v1/recommendations":
            self.reply(200, {"items": PRODUCTS[1:3], "model": "rec-v7"}, delay=(1.15, 1.3))
        elif p == "/static/js/app.9f3c2e.js":
            self.reply(200, JS, "application/javascript", {"Cache-Control": "public, max-age=31536000"}, delay=(0.03, 0.05))
        elif p == "/static/img/logo.png":
            self.reply(200, PNG, "image/png", {"Cache-Control": "public, max-age=86400"})
        elif p == "/api/v1/feature-flags":
            self.reply(304, b"", extra={"ETag": '"ff-20261114"'})
        elif p == "/api/v1/telemetry":
            self.reply(200, PROTO, "application/x-protobuf")
        elif p == "/api/v1/billing/invoices":
            self.reply(500, {"error": "InternalServerError", "message": "Database connection pool exhausted", "code": 50012},
                       delay=(0.25, 0.32))
        elif p.startswith("/api/v1/checkout"):
            self.reply(200, {"status": "confirmed", "orderId": "ord_88231", "path": p})
        else:
            self.reply(404, {"error": "NotFound"})

    def do_POST(self):
        data = self.body()
        p = self.path
        if p == "/api/v1/auth/login":
            self.reply(200, {"accessToken": JWT, "tokenType": "Bearer", "expiresIn": 86400},
                       extra={"Set-Cookie": "sid=s%3A9f2c81d7; Path=/; HttpOnly; Secure; SameSite=Lax"}, delay=(0.12, 0.18))
        elif p == "/api/graphql":
            self.reply(200, GQL, delay=(0.06, 0.12))
        elif p == "/api/v1/orders":
            self.reply(201, {"orderId": "ord_88231", "status": "created", "total": 308.0, "echo": json.loads(data or b"{}")})
        elif p.startswith("/api/v1/checkout"):
            self.reply(200, {"status": "confirmed", "orderId": "ord_88231"})
        else:
            self.reply(200, {"ok": True})

    def do_PUT(self):
        self.body()
        self.reply(200, {"id": "usr_9482", "theme": "dark", "updated": True})

    def do_DELETE(self):
        self.reply(204)

    def log_message(self, *args):
        pass


class Server(socketserver.ThreadingMixIn, socketserver.TCPServer):
    allow_reuse_address = True
    daemon_threads = True


with Server(("127.0.0.1", PORT), Handler) as httpd:
    httpd.serve_forever()
