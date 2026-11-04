# CI packaging fixture

A minimal Cargo binary used by the GitHub Actions packaging smoke test. It intentionally does not
depend on the winia framework so the workflow can validate build and packaging behavior without a
full Skia build.
