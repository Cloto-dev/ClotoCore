import { AlertTriangle, RotateCcw } from 'lucide-react';
import { Component, type ReactNode } from 'react';
import i18n from '../i18n';

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('Cloto ErrorBoundary caught:', error, info.componentStack);
  }

  private async handleRestart() {
    if (import.meta.env.DEV) {
      // Dev: soft restart (Vite may be down, show guidance if reload fails)
      this.setState({ hasError: false, error: null });
      window.location.href = '/';
    } else {
      // Release: full process restart via Tauri
      try {
        const { relaunch } = await import('@tauri-apps/plugin-process');
        await relaunch();
      } catch {
        // Fallback if plugin unavailable
        this.setState({ hasError: false, error: null });
        window.location.href = '/';
      }
    }
  }

  render() {
    if (this.state.hasError) {
      const isDev = import.meta.env.DEV;
      const errorMsg = this.state.error?.message || '';
      const isViteDown = errorMsg.includes('dynamically imported module') || errorMsg.includes('Failed to fetch');

      return (
        <div className="min-h-screen bg-surface-base flex items-center justify-center">
          <div className="text-center space-y-4 max-w-md">
            <div className="mx-auto w-16 h-16 bg-red-500/10 rounded-full flex items-center justify-center border-2 border-red-500/30">
              <AlertTriangle className="text-red-500" size={28} />
            </div>
            <div className="text-xs font-black tracking-[0.3em] text-content-primary uppercase">
              {i18n.t('common:error_boundary_title')}
            </div>
            <p className="text-[10px] font-mono text-content-tertiary px-4 break-all">
              {isDev
                ? this.state.error?.message || i18n.t('common:error_boundary_message')
                : i18n.t('common:error_boundary_message')}
            </p>
            {isDev && isViteDown && (
              <p className="text-[10px] font-mono text-amber-500 px-4">
                Dev server (Vite) may have stopped. Run{' '}
                <code className="bg-surface-secondary px-1 rounded">npx tauri dev</code> in the terminal.
              </p>
            )}
            <button
              onClick={() => this.handleRestart()}
              className="inline-flex items-center gap-2 px-4 py-2 text-xs font-bold uppercase tracking-widest text-white bg-brand rounded hover:bg-[#1e3dd6] transition-colors"
            >
              <RotateCcw size={12} />
              {i18n.t('common:error_boundary_restart')}
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}
