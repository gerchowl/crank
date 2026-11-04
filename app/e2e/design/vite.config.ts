// The design fixtures' own Vite root (#457): serves index.html here, never part of the app build (app/vite.config.ts).
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
export default defineConfig({
  root: import.meta.dirname,
  plugins: [react({ babel: { plugins: ["babel-plugin-react-compiler"] } })],
});
