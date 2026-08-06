import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "localhost",
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
});
