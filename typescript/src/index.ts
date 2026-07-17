/**
 * @hivellm/thunder — HiveLLM binary RPC (wire v1, frozen).
 *
 * Skeleton package: the wire codec + client land at DAG T3.1
 * (`phase3_typescript-package`), corpus-first per SPEC-005.
 */

/** Negotiated wire protocol version. v1 is the only version anywhere. */
export const WIRE_VERSION = 1;

/** Reserved frame id for server push frames (WIRE-005). */
export const PUSH_ID = 0xffff_ffff;

/** Default frame-body cap: 64 MiB, checked before allocation (WIRE-020). */
export const DEFAULT_MAX_FRAME_BYTES = 64 * 1024 * 1024;
