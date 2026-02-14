import { Component, ReactNode } from 'react';
import { AlertTriangle, RotateCcw } from 'lucide-react';

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
    console.error('Exiv ErrorBoundary caught:', error, info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="min-h-screen bg-slate-50 flex items-center justify-center">
          <div className="text-center space-y-4 max-w-md">
            <div className="mx-auto w-16 h-16 bg-red-50 rounded-full flex items-center justify-center border-2 border-red-200">
              <AlertTriangle className="text-red-500" size={28} />
            </div>
            <div className="text-xs font-black tracking-[0.3em] text-slate-800 uppercase">
              System Error
            </div>
            <p className="text-[10px] font-mono text-slate-400 px-4 break-all">
              {this.state.error?.message || 'An unexpected error occurred'}
            </p>
            <button
              onClick={() => {
                this.setState({ hasError: false, error: null });
                window.location.href = '/';
              }}
              className="inline-flex items-center gap-2 px-4 py-2 text-xs font-bold uppercase tracking-widest text-white bg-[#2e4de6] rounded hover:bg-[#1e3dd6] transition-colors"
            >
              <RotateCcw size={12} />
              Restart
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}
