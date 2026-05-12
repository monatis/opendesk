declare module "screenshot-desktop" {
  interface ScreenshotOptions {
    format?: "png" | "jpg";
    screen?: number | string;
    filename?: string;
  }
  function screenshot(options?: ScreenshotOptions): Promise<Buffer>;
  export = screenshot;
}
