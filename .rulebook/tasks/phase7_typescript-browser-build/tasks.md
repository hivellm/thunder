## 1. Implementation
- [ ] 1.1 Verify the claim before designing around it: confirm the wire layer has no transitive Node import, so a wire-only entry is actually achievable without splitting source
- [ ] 1.2 Add a `@hivehub/thunder/wire` subpath export exposing the codec, `FrameReader`, `Value` and the frame helpers — the TypeScript counterpart of the Rust crate's `default-features = false`
- [ ] 1.3 Add a `browser` export condition on the main entry pointing at a client/server-stripped build, so bundlers that do not use the subpath still resolve
- [ ] 1.4 Keep the current entry byte-compatible for Node consumers — nothing existing may break
- [ ] 1.5 Confirm the Node builtins are genuinely absent from the browser artifacts rather than merely tree-shaken away later, since the failure being fixed is a *resolution* error at bundle time

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — `typescript/README.md` gains a browser section naming the subpath, and the root README's package table reflects that the codec is browser-usable
- [ ] 2.2 Write tests covering the new behavior — bundle a two-line browser consumer that imports only the wire subpath and assert the build succeeds with no aliasing and no Node builtin in the output. A test that only imports in Node would not exercise the failure at all
- [ ] 2.3 Run tests and confirm they pass — TypeScript gate green, and the existing Node entry still passes its suite unchanged
