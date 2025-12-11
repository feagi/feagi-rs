# Releasing FEAGI Rust

## Overview

This repository automatically builds and publishes FEAGI binaries for all platforms when you create a version tag.

**Dependencies:** This build requires `feagi-core` crates. The workflow automatically checks out the `fdp-merge-mnt` branch of `feagi-core` during the build (temporary - will use `main` once crates are published).

---

## Release Types

### Production Release
```bash
git tag v1.2.3
git push origin v1.2.3
```
**Publishes:** Stable release, used by feagi-desktop production builds

### Beta Release (Pre-release)
```bash
git tag v1.2.3-beta.1
git push origin v1.2.3-beta.1
```
**Publishes:** Pre-release, used by feagi-desktop beta builds

### Alpha Release (Pre-release)
```bash
git tag v1.2.3-alpha.1
git push origin v1.2.3-alpha.1
```
**Publishes:** Pre-release, typically feagi-desktop uses `staging` branch instead

---

## What Gets Built

When you push a tag, GitHub Actions automatically:

### 1. macOS (Universal Binary)
- ✅ Intel x86_64 + Apple Silicon arm64 combined
- ✅ Single binary works on all Macs
- **Output:** `feagi-macos-v1.2.3.tar.gz`
- Also creates separate arch-specific builds for debugging

### 2. Linux (x86_64)
- ✅ Statically linked (portable)
- **Output:** `feagi-linux-x86_64-v1.2.3.tar.gz`

### 3. Windows (x86_64)
- ✅ Standalone .exe
- **Output:** `feagi-windows-x86_64-v1.2.3.zip`

---

## Package Contents

Each release includes:
- `feagi` binary (or `feagi.exe` on Windows)
- `feagi_configuration.toml`
- `README.txt` with installation instructions

---

## Manual Dispatch

For emergency releases or testing:

**GitHub UI:** Actions → Release FEAGI Binary → Run workflow
- Enter tag name (e.g., `v1.2.3-hotfix`)
- **(Optional)** Override `feagi-core` branch (default: `fdp-merge-mnt`)
  - Current default is `fdp-merge-mnt` (has all required crates)
  - Will switch to `main` once crates are published to crates.io
- Triggers the build manually

---

## Integration with feagi-desktop

### Production/Beta Builds
When feagi-desktop builds a production or beta release:
```yaml
1. Downloads feagi-rust v1.2.3 from GitHub releases
2. Extracts the binary
3. Bundles it into the desktop app
```

**Fast CI:** No compilation needed, just download and bundle (~5 min)

### Alpha/Staging Builds
When feagi-desktop builds alpha or staging:
```yaml
1. Checks out feagi-rust staging branch
2. Compiles from source
3. Bundles the fresh binary
```

**Slower CI:** Full Rust compilation (~10-15 min)

---

## Version Synchronization

### Coordinated Release
For major versions, coordinate tags across repos:

```bash
# feagi-rust
git tag v1.2.3
git push origin v1.2.3

# brain-visualizer
git tag v1.2.3
git push origin v1.2.3

# feagi-desktop (after above releases are published)
git tag v1.2.3
git push origin v1.2.3
```

### Independent Updates
For bug fixes in one component:

```bash
# Just tag the component that changed
cd feagi-rust
git tag v1.2.4
git push origin v1.2.4

# feagi-desktop can still use v1.2.3 of brain-visualizer
```

---

## Troubleshooting

### Build Failed
- Check GitHub Actions logs: Actions → Release FEAGI Binary
- Common issues:
  - Rust compilation errors (fix code)
  - Missing dependencies (update workflow)
  - Network timeouts (retry)

### Release Not Found
- Wait 5-10 minutes for workflow to complete
- Check releases page: https://github.com/neuraville/feagi-rust/releases
- Verify tag was pushed correctly: `git ls-remote --tags origin`

### Wrong Version in Binary
- Ensure Cargo.toml version matches tag
- Recommended: Keep Cargo.toml version generic, rely on Git tags

---

## Best Practices

1. **Test before tagging**: Merge to `main`, verify builds locally
2. **Semantic versioning**: Use v1.2.3 format consistently
3. **Tag annotations**: `git tag -a v1.2.3 -m "Release v1.2.3"`
4. **Changelog**: Update CHANGELOG.md before releasing
5. **Branch protection**: Releases from `main` only (production), `staging` for pre-releases

---

## CI/CD Workflow Details

File: `.github/workflows/release.yml`

**Trigger:** Any tag starting with `v`

**Jobs:**
1. `release-macos` - Builds universal macOS binary
2. `release-linux` - Builds Linux x86_64 binary
3. `release-windows` - Builds Windows x86_64 binary
4. `create-release` - Creates GitHub release with all binaries

**Runtime:** ~10-15 minutes total

**Requirements:**
- Repository has GitHub Actions enabled
- No special secrets needed (uses default GITHUB_TOKEN)
- `feagi/feagi-core` must be accessible (public repo)
- Currently uses `fdp-merge-mnt` branch (will use `main` after crate publication)

---

## For First-Time Setup

1. **Enable GitHub Actions** in repository settings
2. **Test the workflow:**
   ```bash
   git tag v0.1.0-test
   git push origin v0.1.0-test
   ```
3. **Verify release appears** at: https://github.com/neuraville/feagi-rust/releases
4. **Delete test release** if successful

---

## Questions?

- Workflow issues: Check `.github/workflows/release.yml`
- feagi-desktop integration: See `neuraville/feagi-desktop/HYBRID_RELEASE_STRATEGY.md`
- Build problems: Open an issue with Actions logs




## Overview

This repository automatically builds and publishes FEAGI binaries for all platforms when you create a version tag.

**Dependencies:** This build requires `feagi-core` crates. The workflow automatically checks out the `fdp-merge-mnt` branch of `feagi-core` during the build (temporary - will use `main` once crates are published).

---

## Release Types

### Production Release
```bash
git tag v1.2.3
git push origin v1.2.3
```
**Publishes:** Stable release, used by feagi-desktop production builds

### Beta Release (Pre-release)
```bash
git tag v1.2.3-beta.1
git push origin v1.2.3-beta.1
```
**Publishes:** Pre-release, used by feagi-desktop beta builds

### Alpha Release (Pre-release)
```bash
git tag v1.2.3-alpha.1
git push origin v1.2.3-alpha.1
```
**Publishes:** Pre-release, typically feagi-desktop uses `staging` branch instead

---

## What Gets Built

When you push a tag, GitHub Actions automatically:

### 1. macOS (Universal Binary)
- ✅ Intel x86_64 + Apple Silicon arm64 combined
- ✅ Single binary works on all Macs
- **Output:** `feagi-macos-v1.2.3.tar.gz`
- Also creates separate arch-specific builds for debugging

### 2. Linux (x86_64)
- ✅ Statically linked (portable)
- **Output:** `feagi-linux-x86_64-v1.2.3.tar.gz`

### 3. Windows (x86_64)
- ✅ Standalone .exe
- **Output:** `feagi-windows-x86_64-v1.2.3.zip`

---

## Package Contents

Each release includes:
- `feagi` binary (or `feagi.exe` on Windows)
- `feagi_configuration.toml`
- `README.txt` with installation instructions

---

## Manual Dispatch

For emergency releases or testing:

**GitHub UI:** Actions → Release FEAGI Binary → Run workflow
- Enter tag name (e.g., `v1.2.3-hotfix`)
- **(Optional)** Override `feagi-core` branch (default: `fdp-merge-mnt`)
  - Current default is `fdp-merge-mnt` (has all required crates)
  - Will switch to `main` once crates are published to crates.io
- Triggers the build manually

---

## Integration with feagi-desktop

### Production/Beta Builds
When feagi-desktop builds a production or beta release:
```yaml
1. Downloads feagi-rust v1.2.3 from GitHub releases
2. Extracts the binary
3. Bundles it into the desktop app
```

**Fast CI:** No compilation needed, just download and bundle (~5 min)

### Alpha/Staging Builds
When feagi-desktop builds alpha or staging:
```yaml
1. Checks out feagi-rust staging branch
2. Compiles from source
3. Bundles the fresh binary
```

**Slower CI:** Full Rust compilation (~10-15 min)

---

## Version Synchronization

### Coordinated Release
For major versions, coordinate tags across repos:

```bash
# feagi-rust
git tag v1.2.3
git push origin v1.2.3

# brain-visualizer
git tag v1.2.3
git push origin v1.2.3

# feagi-desktop (after above releases are published)
git tag v1.2.3
git push origin v1.2.3
```

### Independent Updates
For bug fixes in one component:

```bash
# Just tag the component that changed
cd feagi-rust
git tag v1.2.4
git push origin v1.2.4

# feagi-desktop can still use v1.2.3 of brain-visualizer
```

---

## Troubleshooting

### Build Failed
- Check GitHub Actions logs: Actions → Release FEAGI Binary
- Common issues:
  - Rust compilation errors (fix code)
  - Missing dependencies (update workflow)
  - Network timeouts (retry)

### Release Not Found
- Wait 5-10 minutes for workflow to complete
- Check releases page: https://github.com/neuraville/feagi-rust/releases
- Verify tag was pushed correctly: `git ls-remote --tags origin`

### Wrong Version in Binary
- Ensure Cargo.toml version matches tag
- Recommended: Keep Cargo.toml version generic, rely on Git tags

---

## Best Practices

1. **Test before tagging**: Merge to `main`, verify builds locally
2. **Semantic versioning**: Use v1.2.3 format consistently
3. **Tag annotations**: `git tag -a v1.2.3 -m "Release v1.2.3"`
4. **Changelog**: Update CHANGELOG.md before releasing
5. **Branch protection**: Releases from `main` only (production), `staging` for pre-releases

---

## CI/CD Workflow Details

File: `.github/workflows/release.yml`

**Trigger:** Any tag starting with `v`

**Jobs:**
1. `release-macos` - Builds universal macOS binary
2. `release-linux` - Builds Linux x86_64 binary
3. `release-windows` - Builds Windows x86_64 binary
4. `create-release` - Creates GitHub release with all binaries

**Runtime:** ~10-15 minutes total

**Requirements:**
- Repository has GitHub Actions enabled
- No special secrets needed (uses default GITHUB_TOKEN)
- `feagi/feagi-core` must be accessible (public repo)
- Currently uses `fdp-merge-mnt` branch (will use `main` after crate publication)

---

## For First-Time Setup

1. **Enable GitHub Actions** in repository settings
2. **Test the workflow:**
   ```bash
   git tag v0.1.0-test
   git push origin v0.1.0-test
   ```
3. **Verify release appears** at: https://github.com/neuraville/feagi-rust/releases
4. **Delete test release** if successful

---

## Questions?

- Workflow issues: Check `.github/workflows/release.yml`
- feagi-desktop integration: See `neuraville/feagi-desktop/HYBRID_RELEASE_STRATEGY.md`
- Build problems: Open an issue with Actions logs

