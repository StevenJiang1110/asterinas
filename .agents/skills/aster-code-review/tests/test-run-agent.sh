#!/usr/bin/env bash
# Tests for scripts/run_agent.sh profile/config handling.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
RUN_AGENT="$HERE/../scripts/run_agent.sh"

setup() {
    mkdir -p "$TMP/home/.codex" "$TMP/profile"
    cat > "$TMP/home/.codex/config.toml" <<'EOF'
model_provider = "local-provider"
model = "from-home"
model_reasoning_effort = "medium"

[model_providers.local-provider]
name = "Local Provider"
base_url = "https://example.test/v1"
requires_openai_auth = false
wire_api = "responses"
EOF
    cat > "$TMP/profile/config.toml" <<'EOF'
model = "profile-model"
model_reasoning_effort = "high"
sandbox_mode = "danger-full-access"
approval_policy = "never"
EOF
    cat > "$TMP/profile/config.smoke.toml" <<'EOF'
model = "smoke-model"
model_reasoning_effort = "low"
EOF
    cat > "$TMP/profile/profile.json" <<'EOF'
{
  "config_base": "{home}/.codex/config.toml",
  "env": { "CODEX_HOME": "{workdir}" },
  "command": ["sh", "-c", "cat \"$CODEX_HOME/config.toml\""]
}
EOF
}

test_config_base_preserves_provider_tables() {
    local out
    out="$(HOME="$TMP/home" ACR_AGENT_PROFILE="$TMP/profile" "$RUN_AGENT" "ignored")"
    assert_contains "keeps provider selection" "$out" 'model_provider = "local-provider"'
    assert_contains "keeps provider table" "$out" '[model_providers.local-provider]'
    assert_contains "profile overrides model" "$out" 'model = "profile-model"'
    assert_absent "home model removed" "$out" 'model = "from-home"'
    assert_contains "profile adds approval" "$out" 'approval_policy = "never"'
}

test_smoke_overlay_overrides_profile_keys() {
    local out
    out="$(HOME="$TMP/home" ACR_AGENT_PROFILE="$TMP/profile" ACR_PROFILE_VARIANT=smoke "$RUN_AGENT" "ignored")"
    assert_contains "smoke overrides model" "$out" 'model = "smoke-model"'
    assert_contains "smoke overrides effort" "$out" 'model_reasoning_effort = "low"'
    assert_contains "keeps provider table" "$out" '[model_providers.local-provider]'
}

run_suite
