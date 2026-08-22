export PNPM_HOME="$HOME/Library/pnpm"
case ":$PATH:" in
  *":$PNPM_HOME/bin:"*) ;;
  *) path=("$PNPM_HOME/bin" $path) ;;
esac
