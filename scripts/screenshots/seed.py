"""Demo data for the README screenshots: a small online shop.

    python -I scripts/screenshots/seed.py <folder>

Writes <folder>/shop.db (SQLite: tables, a view, a trigger, a few hundred rows) and
<folder>/orders.js (mongosh insertMany statements for a `tableboat_demo` MongoDB database).
Everything is made up and deterministic (fixed random seed), so screenshots can be redone.
"""
import json
import random
import sqlite3
import sys
from datetime import datetime, timedelta
from pathlib import Path

rnd = random.Random(7)
out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)

FIRST = ["Ada", "Grace", "Alan", "Katherine", "Linus", "Margaret", "Dennis", "Barbara", "Ken", "Radia", "Edsger",
         "Frances", "Donald", "Hedy", "Tim", "Sophie", "John", "Annie", "Guido", "Evelyn", "Niklaus", "Jean",
         "Bjarne", "Mary", "Brian", "Joan", "Anders", "Lynn", "James", "Shafi"]
LAST = ["Lovelace", "Hopper", "Turing", "Johnson", "Torvalds", "Hamilton", "Ritchie", "Liskov", "Thompson",
        "Perlman", "Dijkstra", "Allen", "Knuth", "Lamarr", "Berners-Lee", "Wilson", "McCarthy", "Easley",
        "van Rossum", "Boyd", "Wirth", "Sammet", "Stroustrup", "Keller", "Kernighan", "Clarke", "Hejlsberg",
        "Conway", "Gosling", "Goldwasser"]
CITIES = [("Lisbon", "PT"), ("Berlin", "DE"), ("Austin", "US"), ("Pune", "IN"), ("Toronto", "CA"), ("Osaka", "JP"),
          ("Melbourne", "AU"), ("Nairobi", "KE"), ("Utrecht", "NL"), ("Bogotá", "CO"), ("Oslo", "NO"), ("Seoul", "KR")]
CATEGORIES = ["Keyboards", "Mice", "Monitors", "Audio", "Cables", "Desks", "Lighting", "Storage"]
PRODUCTS = [
    ("Keyboards", "Low-profile mechanical keyboard", 129.00), ("Keyboards", "Split ergonomic keyboard", 249.00),
    ("Keyboards", "Wireless compact keyboard", 79.50), ("Mice", "Vertical mouse", 59.90), ("Mice", "Trackball", 89.00),
    ("Mice", "Silent office mouse", 24.99), ("Monitors", "27\" 4K monitor", 429.00), ("Monitors", "34\" ultrawide", 599.00),
    ("Monitors", "Portable 15\" display", 189.00), ("Audio", "Noise-cancelling headset", 199.00),
    ("Audio", "USB condenser microphone", 119.00), ("Audio", "Desk speakers", 149.00), ("Cables", "USB-C hub, 8 ports", 69.00),
    ("Cables", "Thunderbolt cable 2 m", 39.00), ("Cables", "HDMI 2.1 cable", 19.90), ("Desks", "Standing desk frame", 389.00),
    ("Desks", "Monitor arm", 99.00), ("Desks", "Cable tray", 29.00), ("Lighting", "Monitor light bar", 59.00),
    ("Lighting", "Desk lamp, warm white", 45.00), ("Storage", "2 TB NVMe SSD", 159.00), ("Storage", "4-bay NAS", 549.00),
    ("Storage", "USB flash drive 256 GB", 29.99), ("Audio", "Studio headphones", 169.00)]
STATUS = ["paid"] * 7 + ["shipped"] * 6 + ["pending"] * 2 + ["refunded"]

db_path = out / "shop.db"
db_path.unlink(missing_ok=True)
db = sqlite3.connect(db_path)
db.executescript("""
CREATE TABLE categories (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE
);
CREATE TABLE products (
  id INTEGER PRIMARY KEY,
  category_id INTEGER NOT NULL REFERENCES categories(id),
  name TEXT NOT NULL,
  price DECIMAL(10,2) NOT NULL,
  in_stock INTEGER NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL
);
CREATE TABLE customers (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  email TEXT NOT NULL UNIQUE,
  city TEXT,
  country CHAR(2),
  vip BOOLEAN NOT NULL DEFAULT 0,
  note TEXT,
  created_at DATETIME NOT NULL
);
CREATE TABLE orders (
  id INTEGER PRIMARY KEY,
  customer_id INTEGER NOT NULL REFERENCES customers(id),
  status TEXT NOT NULL CHECK (status IN ('pending','paid','shipped','refunded')),
  total DECIMAL(10,2) NOT NULL,
  created_at DATETIME NOT NULL
);
CREATE TABLE order_items (
  order_id INTEGER NOT NULL REFERENCES orders(id),
  product_id INTEGER NOT NULL REFERENCES products(id),
  quantity INTEGER NOT NULL,
  unit_price DECIMAL(10,2) NOT NULL,
  PRIMARY KEY (order_id, product_id)
);
CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY,
  order_id INTEGER NOT NULL,
  old_status TEXT,
  new_status TEXT,
  changed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX orders_customer ON orders(customer_id);
CREATE INDEX orders_created ON orders(created_at);
CREATE VIEW revenue_by_customer AS
  SELECT c.id, c.name, c.country, COUNT(o.id) AS orders, ROUND(SUM(o.total), 2) AS revenue
  FROM customers c JOIN orders o ON o.customer_id = c.id
  WHERE o.status <> 'refunded'
  GROUP BY c.id;
CREATE VIEW monthly_sales AS
  SELECT strftime('%Y-%m', created_at) AS month, COUNT(*) AS orders, ROUND(SUM(total), 2) AS revenue
  FROM orders WHERE status <> 'refunded' GROUP BY month;
CREATE TRIGGER orders_status_audit AFTER UPDATE OF status ON orders
BEGIN
  INSERT INTO audit_log (order_id, old_status, new_status) VALUES (old.id, old.status, new.status);
END;
""")

start = datetime(2026, 5, 1, 9, 0)
for i, name in enumerate(CATEGORIES, 1):
    db.execute("INSERT INTO categories VALUES (?, ?)", (i, name))
for i, (cat, name, price) in enumerate(PRODUCTS, 1):
    db.execute("INSERT INTO products VALUES (?, ?, ?, ?, ?, ?)",
               (i, CATEGORIES.index(cat) + 1, name, price, rnd.randint(0, 240), (start - timedelta(days=rnd.randint(30, 400))).isoformat(" ")))

NOTES = [None] * 6 + ["prefers email invoices", "gift wrap", "business account", "call before delivery"]
customers = []
for i in range(1, 49):
    first, last = FIRST[(i * 7) % len(FIRST)], LAST[(i * 11) % len(LAST)]
    city, country = CITIES[i % len(CITIES)]
    email = f"{first}.{last}".lower().replace(" ", "").replace("-", "") + f"{i}@example.com"
    customers.append((i, f"{first} {last}"))
    db.execute("INSERT INTO customers VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
               (i, f"{first} {last}", email, city, country, int(rnd.random() < 0.18), rnd.choice(NOTES),
                (start - timedelta(days=rnd.randint(5, 500))).isoformat(" ")))

mongo_orders = []
for oid in range(1, 361):
    cust = rnd.choice(customers)
    when = start + timedelta(days=rnd.randint(0, 196), minutes=rnd.randint(0, 600))
    items = rnd.sample(range(1, len(PRODUCTS) + 1), rnd.randint(1, 3))
    total = 0.0
    lines = []
    for pid in items:
        qty = rnd.choice([1, 1, 1, 2, 3])
        price = PRODUCTS[pid - 1][2]
        total += qty * price
        db.execute("INSERT INTO order_items VALUES (?, ?, ?, ?)", (oid, pid, qty, price))
        lines.append({"sku": f"SKU-{pid:04d}", "name": PRODUCTS[pid - 1][1], "qty": qty, "price": price})
    status = rnd.choice(STATUS)
    db.execute("INSERT INTO orders VALUES (?, ?, ?, ?, ?)", (oid, cust[0], status, round(total, 2), when.isoformat(" ")))
    if oid > 340:
        city, country = CITIES[cust[0] % len(CITIES)]
        mongo_orders.append((oid, cust[1], city, country, status, round(total, 2), when, lines))
db.commit()
db.close()

# MongoDB: the last 20 orders as documents, with nested items and real BSON types.
docs = []
for oid, name, city, country, status, total, when, lines in mongo_orders:
    items = ", ".join(
        "{ sku: %s, name: %s, qty: %d, price: NumberDecimal(%s) }" % (json.dumps(l["sku"]), json.dumps(l["name"]), l["qty"], json.dumps(f"{l['price']:.2f}"))
        for l in lines)
    tags = json.dumps(rnd.sample(["gift", "express", "b2b", "first-order", "repeat"], rnd.randint(0, 2)))
    docs.append(
        "{ orderNo: NumberLong(%d), customer: { name: %s, city: %s, country: %s }, status: %s, total: NumberDecimal(%s), "
        "items: [%s], tags: %s, placedAt: ISODate(%s), paid: %s }"
        % (100000 + oid, json.dumps(name), json.dumps(city), json.dumps(country), json.dumps(status), json.dumps(f"{total:.2f}"),
           items, tags, json.dumps(when.strftime("%Y-%m-%dT%H:%M:%SZ")), "true" if status != "pending" else "false"))
(out / "orders.js").write_text("db.orders.insertMany([" + ", ".join(docs) + "])", encoding="utf-8")
print(f"wrote {db_path} and {out / 'orders.js'}")
