import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** @type {import("@sveltejs/vite-plugin-svelte").SvelteConfig} */
export default {
  preprocess: vitePreprocess(),
  compilerOptions: {
    // Svelte 4 syntax (export let, $:, on:click, <slot>) is a hard compile error.
    runes: true,
  },
};
