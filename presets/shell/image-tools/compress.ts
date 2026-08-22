// Compress JPEG, PNG, and WebP images without modifying the originals.
import { runImageTool } from "./image-tools.ts";

if (import.meta.main) {
  process.exit(await runImageTool("compress"));
}
