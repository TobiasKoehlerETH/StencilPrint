import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  define: { __APP_VERSION__: JSON.stringify(process.env.npm_package_version ?? "0.0.0") },
  envPrefix: ["VITE_", "TAURI_"],
  plugins: [react()],
  server: { port: 1420, strictPort: true },
});
