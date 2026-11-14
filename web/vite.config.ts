import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// `npm run dev` talks to a running agent-duo (cargo run -p agent-duo -- --port 3000 --no-open).
export default defineConfig({
  plugins: [react()],
  server: { port: 5173, proxy: { "/api": "http://127.0.0.1:3000" } },
});
