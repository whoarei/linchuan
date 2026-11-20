import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "vite";

const root = fileURLToPath(new URL("../", import.meta.url));
const previewDir = new URL("../prototype/dist/", import.meta.url);
const { output } = await build({
  root,
  build: {
    write: false,
    cssCodeSplit: false,
    modulePreload: false,
    rollupOptions: {
      input: fileURLToPath(new URL("../prototype/remote-assistance.html", import.meta.url)),
      output: { inlineDynamicImports: true },
    },
  },
});

const html = output.find((file) => file.type === "asset" && file.fileName.endsWith(".html"));
const css = output.find((file) => file.type === "asset" && file.fileName.endsWith(".css"));
const script = output.find((file) => file.type === "chunk" && file.isEntry);
const scriptTag = /<script\b[^>]*\bsrc="[^"]+"[^>]*><\/script>/;
const styleTag = /<link\b[^>]*\brel="stylesheet"[^>]*>/;

if (!html || !css || !script || !scriptTag.test(html.source) || !styleTag.test(html.source)) {
  throw new Error("The preview build must emit HTML with one script and one stylesheet.");
}

const inlineScript = script.code.replace(/<\/script/gi, "<\\/script");
const preview = html.source
  .replace(scriptTag, () => `<script type="module">${inlineScript}</script>`)
  .replace(styleTag, () => `<style>${css.source}</style>`);

await mkdir(previewDir, { recursive: true });
await writeFile(new URL("remote-assistance.html", previewDir), preview);
console.log("Preview saved to prototype/dist/remote-assistance.html");
