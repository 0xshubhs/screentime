import { defineConfig } from "vite";

// `npm run dev` proxies the API to a running screentimed.
export default defineConfig({
  server: {
    port: 5173,
    proxy: { "/api": process.env.SCREENTIME_API ?? "http://127.0.0.1:47821" },
  },
  build: { outDir: "dist", emptyOutDir: true },
});
