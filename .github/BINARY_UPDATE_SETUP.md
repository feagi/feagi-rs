# Binary Update Workflow Setup

This document explains how to set up the automated binary update workflow that builds FEAGI binaries and creates PRs in the public `feagi/feagi` repository.

## Overview

When a PR is successfully merged to the `staging` branch in `FEAGI-2.0/feagi`, this workflow:
1. Builds FEAGI binaries for all 5 platforms (Linux x86_64, Linux ARM64, macOS Intel, macOS ARM64, Windows)
2. Creates a new branch in the public `feagi/feagi` repository
3. Commits the binaries to `feagi/bin/` folder
4. Creates a Pull Request for review and approval

## Required GitHub Token Setup

### Step 1: Create a Personal Access Token (PAT) or GitHub App Token

You have two options:

#### Option A: Personal Access Token (PAT) - Recommended for simplicity

1. Go to GitHub Settings → Developer settings → Personal access tokens → Tokens (classic)
2. Click "Generate new token (classic)"
3. Give it a descriptive name: `FEAGI Binary Update Bot`
4. Set expiration (recommend: 90 days or custom)
5. Select the following scopes:
   - ✅ `repo` (Full control of private repositories)
     - This includes:
       - `repo:status` - Access commit status
       - `repo_deployment` - Access deployment status
       - `public_repo` - Access public repositories
       - `repo:invite` - Access repository invitations
       - `security_events` - Read and write security events
6. Click "Generate token"
7. **Copy the token immediately** (you won't be able to see it again)

#### Option B: GitHub App (More secure, recommended for organizations)

1. Go to your organization settings → GitHub Apps
2. Create a new GitHub App
3. Set permissions:
   - Repository permissions:
     - Contents: Read and write
     - Pull requests: Read and write
     - Metadata: Read-only
4. Install the app on both repositories:
   - `FEAGI-2.0/feagi` (private)
   - `feagi/feagi` (public)
5. Generate a private key and use it to create installation tokens

### Step 2: Add Token as Repository Secret

1. Go to `FEAGI-2.0/feagi` repository (private repo)
2. Navigate to: Settings → Secrets and variables → Actions
3. Click "New repository secret"
4. Name: `FEAGI_PUBLIC_REPO_TOKEN`
5. Value: Paste your token from Step 1
6. Click "Add secret"

### Step 3: Verify Token Permissions

The token needs the following permissions:
- ✅ **Contents: Write** - To create branches, commit, and push
- ✅ **Pull Requests: Write** - To create PRs
- ✅ **Metadata: Read** - Basic repository access

## Workflow Behavior

### Trigger Conditions

The workflow triggers when:
- A PR is **merged** (not just closed) to the `staging` branch
- The PR targets the `staging` branch
- Changes affect Rust source files, `Cargo.toml`, or the workflow itself

### Build Process

1. **Matrix Build**: Builds run in parallel on 5 different runners:
   - `ubuntu-latest` for Linux x86_64
   - `ubuntu-latest` for Linux ARM64
   - `macos-13` for macOS Intel
   - `macos-latest` for macOS ARM64
   - `windows-latest` for Windows x86_64

2. **Binary Collection**: All binaries are uploaded as artifacts

3. **PR Creation**: A single job collects all binaries and:
   - Checks out the public `feagi/feagi` repository
   - Creates a branch: `update-binaries-YYYYMMDD-HHMMSS-<sha>`
   - Copies binaries to `feagi/bin/<platform>/`
   - Commits with a descriptive message
   - Pushes the branch
   - Creates a PR with labels: `automated`, `binaries`

### Branch Naming

Branches are named: `update-binaries-<timestamp>-<short-sha>`

Example: `update-binaries-20251120-143022-a1b2c3d`

### PR Format

Each PR includes:
- Title: "chore: Update FEAGI binaries from staging"
- Body with:
  - List of included binaries
  - Source commit SHA
  - Source repository and branch
  - Triggering PR number
- Labels: `automated`, `binaries`

## Troubleshooting

### Workflow doesn't trigger

- ✅ Ensure PR was **merged** (not just closed)
- ✅ Verify PR targets `staging` branch
- ✅ Check that changes affect Rust files or `Cargo.toml`

### "Permission denied" errors

- ✅ Verify `FEAGI_PUBLIC_REPO_TOKEN` secret is set correctly
- ✅ Check token has `repo` scope (for PAT) or correct app permissions
- ✅ Ensure token has access to both repositories

### "No changes to commit"

- This is normal if binaries are identical to existing ones
- The workflow will exit successfully without creating a PR

### Build failures

- Check individual build job logs
- Common issues:
  - Missing Rust toolchain targets
  - Dependency compilation errors
  - Platform-specific build issues

## Manual Trigger (if needed)

To manually trigger the workflow:

1. Create a dummy PR to `staging`
2. Merge it
3. The workflow will run automatically

Or use GitHub CLI:
```bash
gh workflow run build-and-push-binaries.yml
```

## Security Considerations

- ✅ Token is stored as a repository secret (encrypted)
- ✅ Token only has necessary permissions
- ✅ PRs require manual approval before merging
- ✅ All actions are logged in GitHub Actions

## Maintenance

- **Token Rotation**: Update `FEAGI_PUBLIC_REPO_TOKEN` secret when token expires
- **Workflow Updates**: Modify `.github/workflows/build-and-push-binaries.yml` as needed
- **Monitoring**: Check Actions tab for workflow runs and failures

