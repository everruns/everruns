CREATE TABLE IF NOT EXISTS orders (id INTEGER PRIMARY KEY, region TEXT NOT NULL, amount_cents INTEGER NOT NULL);
INSERT OR REPLACE INTO orders VALUES (1, 'north', 18037), (2, 'north', 9241), (3, 'south', 3500);
