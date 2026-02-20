#!/bin/bash
# Install Aperture shell helpers

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Resolve the Aperture root (parent of scripts/)
APERTURE_DIR="$(dirname "$SCRIPT_DIR")"

echo "Installing Aperture shell helpers..."

# Detect shell
if [ -n "$FISH_VERSION" ] || [ -d ~/.config/fish ]; then
    echo "→ Installing fish function"
    mkdir -p ~/.config/fish/functions
    # Substitute the actual install path so 'aperture start' works correctly
    sed "s|\$HOME/projects/Aperture|$APERTURE_DIR|g" "$SCRIPT_DIR/aperture.fish" > ~/.config/fish/functions/aperture.fish
    echo "  Installed to ~/.config/fish/functions/aperture.fish (path: $APERTURE_DIR)"
fi

if [ -f ~/.bashrc ]; then
    echo "→ Adding bash alias to ~/.bashrc"
    if ! grep -q "aperture()" ~/.bashrc; then
        cat >> ~/.bashrc << EOF

# Aperture - Universal LLM Context Proxy
aperture() {
    local APERTURE_URL="http://localhost:5400"
    case "\$1" in
        ""|help|-h|--help)
            echo "Usage: aperture <tool> [args...] - Launch tool through Aperture proxy"
            echo "Tools: claude, codex, aider, opencode, or any command"
            ;;
        status)
            nc -z localhost 5400 && echo "✓ Aperture running" || echo "✗ Aperture not running"
            ;;
        start)
            (cd "$APERTURE_DIR" && npm run tauri dev &)
            ;;
        claude|claude-code)
            shift; ANTHROPIC_BASE_URL="\$APERTURE_URL" claude "\$@"
            ;;
        codex)
            shift; OPENAI_BASE_URL="\$APERTURE_URL" codex "\$@"
            ;;
        *)
            ANTHROPIC_BASE_URL="\$APERTURE_URL" OPENAI_BASE_URL="\$APERTURE_URL" "\$@"
            ;;
    esac
}
EOF
        echo "  Added aperture function to ~/.bashrc (path: $APERTURE_DIR)"
    else
        echo "  Already in ~/.bashrc"
    fi
fi

if [ -f ~/.zshrc ]; then
    echo "→ Adding zsh alias to ~/.zshrc"
    if ! grep -q "aperture()" ~/.zshrc; then
        cat >> ~/.zshrc << EOF

# Aperture - Universal LLM Context Proxy
aperture() {
    local APERTURE_URL="http://localhost:5400"
    case "\$1" in
        ""|help|-h|--help)
            echo "Usage: aperture <tool> [args...] - Launch tool through Aperture proxy"
            echo "Tools: claude, codex, aider, opencode, or any command"
            ;;
        status)
            nc -z localhost 5400 && echo "✓ Aperture running" || echo "✗ Aperture not running"
            ;;
        start)
            (cd "$APERTURE_DIR" && npm run tauri dev &)
            ;;
        claude|claude-code)
            shift; ANTHROPIC_BASE_URL="\$APERTURE_URL" claude "\$@"
            ;;
        codex)
            shift; OPENAI_BASE_URL="\$APERTURE_URL" codex "\$@"
            ;;
        *)
            ANTHROPIC_BASE_URL="\$APERTURE_URL" OPENAI_BASE_URL="\$APERTURE_URL" "\$@"
            ;;
    esac
}
EOF
        echo "  Added aperture function to ~/.zshrc (path: $APERTURE_DIR)"
    else
        echo "  Already in ~/.zshrc"
    fi
fi

echo ""
echo "Done! Restart your shell or run:"
echo "  source ~/.config/fish/config.fish  # fish"
echo "  source ~/.bashrc                   # bash"
echo "  source ~/.zshrc                    # zsh"
echo ""
echo "Then try: aperture status"
