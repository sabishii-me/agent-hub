# The plugin release workflow (admin-tagged, CI-built artifact)

The owner's decision: **an ADMIN pushes a version tag** (`v<version>`). That tag is the ONLY trigger.
CI then runs the plugin release flow and publishes the artifact. Nothing runs on ordinary commits, so
CI cost is one small job per release.

## The flow (per plugin repo)

```
admin:  git tag v<version> && git push origin v<version>
   -> GitHub Actions (on: push tags 'v*')
        1. checkout
        2. assert the tag matches manifest.json version (a tag that names a version the manifest
           does not is REFUSED - the artifact would lie)
        3. zip the git-tracked files (excludes runtime/, gitignored and materialised by the
           adapter's runtime/prepare at INSTALL time) -> harness-adapter-<id>-<version>.zip
        4. sha256 the zip
        5. gh release create <tag> --title "<id> <version>" \
             --notes "Plugin artifact for <id> <version> (sha256 <hash>)." \
             harness-adapter-<id>-<version>.zip
```

## CI cost (measured)

Building is zip + sha256 of 11-17 tracked files. Measured on the three repos:

| plugin | files | zip | build |
|---|---|---|---|
| pi 0.1.10 | 17 | 56 KB | 11 ms |
| jouzu 0.1.11 | 17 | 637 KB | 28 ms |
| deepseek 0.1.8 | 11 | 88 KB | 12 ms |

No npm, no runtime, no large build in CI: the runtime is installed at INSTALL time by the adapter,
not at release time. So one release job is seconds and a sub-1MB upload - negligible CI cost.

## The workflow file (goes in EACH plugin repo at .github/workflows/release.yml on main)

```yaml
name: release
on:
  push:
    tags: ['v*']
permissions:
  contents: write
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Read identity + assert the tag matches the manifest
        id: id
        run: |
          set -euo pipefail
          id=$(python3 -c "import json;print(json.load(open('manifest.json'))['id'])")
          ver=$(python3 -c "import json;print(json.load(open('manifest.json'))['version'])")
          tag="${GITHUB_REF_NAME#v}"
          if [ "$tag" != "$ver" ]; then
            echo "tag $GITHUB_REF_NAME does not match manifest version $ver" >&2
            exit 1
          fi
          echo "id=$id"   >> "$GITHUB_OUTPUT"
          echo "ver=$ver" >> "$GITHUB_OUTPUT"
          echo "zip=harness-adapter-$id-$ver.zip" >> "$GITHUB_OUTPUT"
      - name: Build the artifact (tracked files only; runtime/ is gitignored)
        run: |
          set -euo pipefail
          zip -r "${{ steps.id.outputs.zip }}" $(git ls-files)
      - name: sha256
        id: sha
        run: echo "hash=$(sha256sum '${{ steps.id.outputs.zip }}' | cut -d' ' -f1)" >> "$GITHUB_OUTPUT"
      - name: Publish the release
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          set -euo pipefail
          gh release create "$GITHUB_REF_NAME" \
            --title "${{ steps.id.outputs.id }} ${{ steps.id.outputs.ver }}" \
            --notes "Plugin artifact for ${{ steps.id.outputs.id }} ${{ steps.id.outputs.ver }} (sha256 ${{ steps.sha.outputs.hash }})." \
            "${{ steps.id.outputs.zip }}"
```

## Preconditions before the first tag

- the change is COMMITTED on `main` (the workflow must exist on the default branch for a tag event);
- the manifest version (and package.json version) is the version being tagged;
- for deepseek: the adapter fix (do not die at startup; runtime/prepare must be reachable) is
  committed first (docs/issues/20261008-005306).
