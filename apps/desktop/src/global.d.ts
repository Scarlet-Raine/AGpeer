export {};

declare global {
  interface Window {
    /** Injected by the desktop shell when the core token is available. */
    __AGPEER_TOKEN__?: string;
  }
}
