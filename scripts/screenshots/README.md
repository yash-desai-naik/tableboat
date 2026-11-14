# README screenshots

The screenshots in `docs/screenshots/` are of the real app, driven through WebView2's DevTools
protocol, on made-up data. To redo them:

1. **Demo data.** `python -I scripts/screenshots/seed.py <folder>` writes `shop.db` (SQLite: 48
   customers, 360 orders, views and a trigger) and `orders.js` (20 MongoDB order documents).
   Copy `shop.db` somewhere with a short, neutral path (it shows in the connection list).
2. **A separate profile**, so your own connections never appear:
   ```powershell
   $env:TABLEBOAT_DATA_DIR = "<folder>\profile"
   tb conn add "sqlite:///D:/demo/shop.db" --name Shop --tag local --color "#16a34a"
   tb conn add "mongodb://127.0.0.1:27117/tableboat_demo" --name Orders --tag development
   tb query Orders -i <folder>\orders.js
   ```
   For MongoDB, any throwaway `mongod --dbpath <folder>\mongo --port 27117` works.
3. **Start Tableboat with DevTools** (close any running copy first; only one can run):
   ```powershell
   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
   & "$env:LOCALAPPDATA\Tableboat\tableboat.exe"
   ```
4. **Drive and capture** with `node scripts/cdp.mjs steps.json`. Each run is a new DevTools session,
   so start every steps file with the viewport, e.g.
   `{"cdp": "Emulation.setDeviceMetricsOverride", "params": {"width": 1440, "height": 900, "deviceScaleFactor": 1.5, "mobile": false}}`,
   then `click`, `drag`, `key`, `text`, `wait` and `shot` steps. Switch themes through the command
   palette (Ctrl+Shift+P, then the theme name).

Check every image before committing: no real connection names, paths or user names.
