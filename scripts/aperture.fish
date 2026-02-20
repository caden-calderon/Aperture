# Aperture - Universal LLM Context Proxy
# Add to ~/.config/fish/functions/aperture.fish or source in config.fish

function aperture --description "Launch AI tools through Aperture proxy"
    set -l APERTURE_PORT 5400
    set -l APERTURE_URL "http://localhost:$APERTURE_PORT"
    # Override APERTURE_DIR if Aperture is not installed at ~/projects/Aperture
    set -l APERTURE_DIR (set -q APERTURE_DIR; and echo $APERTURE_DIR; or echo "$HOME/projects/Aperture")

    # Check if Aperture is running
    if not nc -z localhost $APERTURE_PORT 2>/dev/null
        echo "⚠ Aperture proxy not running on port $APERTURE_PORT"
        echo "  Start it with: cd ~/projects/Aperture && npm run tauri dev"
        echo ""
    end

    switch "$argv[1]"
        case "" "help" "-h" "--help"
            echo "Aperture - Universal LLM Context Proxy"
            echo ""
            echo "Usage:"
            echo "  aperture <tool> [args...]   Launch tool through proxy"
            echo "  aperture status             Check proxy status"
            echo "  aperture start              Start Aperture app"
            echo ""
            echo "Supported tools:"
            echo "  claude, claude-code         Claude Code CLI"
            echo "  codex                       OpenAI Codex CLI"
            echo "  aider                       Aider"
            echo "  opencode                    OpenCode"
            echo "  cursor                      Cursor (if CLI available)"
            echo ""
            echo "Examples:"
            echo "  aperture claude             Start Claude Code with proxy"
            echo "  aperture claude -p .        Start in current directory"
            echo "  aperture codex              Start Codex with proxy"
            echo ""

        case "status"
            if nc -z localhost $APERTURE_PORT 2>/dev/null
                echo "✓ Aperture proxy running on $APERTURE_URL"
            else
                echo "✗ Aperture proxy not running"
            end

        case "start"
            echo "Starting Aperture..."
            cd $APERTURE_DIR && npm run tauri dev &
            disown
            echo "Aperture starting in background. Wait a few seconds then run your tool."

        case "claude" "claude-code"
            echo "→ Launching Claude Code through Aperture"
            if test (count $argv) -eq 1
                ANTHROPIC_BASE_URL=$APERTURE_URL claude
            else
                ANTHROPIC_BASE_URL=$APERTURE_URL claude $argv[2..-1]
            end

        case "codex"
            echo "→ Launching Codex through Aperture"
            if test (count $argv) -eq 1
                OPENAI_BASE_URL=$APERTURE_URL codex
            else
                OPENAI_BASE_URL=$APERTURE_URL codex $argv[2..-1]
            end

        case "aider"
            echo "→ Launching Aider through Aperture"
            if test (count $argv) -eq 1
                ANTHROPIC_BASE_URL=$APERTURE_URL OPENAI_BASE_URL=$APERTURE_URL aider
            else
                ANTHROPIC_BASE_URL=$APERTURE_URL OPENAI_BASE_URL=$APERTURE_URL aider $argv[2..-1]
            end

        case "opencode"
            echo "→ Launching OpenCode through Aperture"
            if test (count $argv) -eq 1
                ANTHROPIC_BASE_URL=$APERTURE_URL OPENAI_BASE_URL=$APERTURE_URL opencode
            else
                ANTHROPIC_BASE_URL=$APERTURE_URL OPENAI_BASE_URL=$APERTURE_URL opencode $argv[2..-1]
            end

        case "*"
            # Try to run any command with both env vars set
            echo "→ Launching $argv[1] through Aperture"
            ANTHROPIC_BASE_URL=$APERTURE_URL OPENAI_BASE_URL=$APERTURE_URL $argv
    end
end
