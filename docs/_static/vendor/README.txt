Matter.js 0.20.0
https://github.com/liabru/matter-js

matter.min.js is the unmodified build/matter.min.js from the npm package
matter-js@0.20.0. LICENSE is the upstream license from the same package.
Used only by the documentation background, not by the native library.

Refresh from the pinned npm tarball, retaining the upstream license.
The animation adapter lowers Resolver._restingThresh to 0.001 to retain
elastic contacts at decorative drift speeds. This internal setting is tied
to 0.20.0; verify scripts/test-docs-particles.cjs when updating Matter.js.