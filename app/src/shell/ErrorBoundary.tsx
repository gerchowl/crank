// Top-level boundary (#88; spike defect #4): a throwing view shows what broke and the way out, never a white
// window. Keyed by route in the shell, so navigating also clears it.
import { Component, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  /** "Back to Today": the shell navigates; the boundary clears its own error. */
  onBack: () => void;
}

export class ErrorBoundary extends Component<Props, { error: Error | null }> {
  override state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  override render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="page" role="alert">
        <h1>Something went wrong</h1>
        <p className="muted">{error.message}</p>
        <p className="muted">Your rides and settings were not touched.</p>
        <button
          type="button"
          className="primary"
          onClick={() => {
            this.setState({ error: null });
            this.props.onBack();
          }}
        >
          Back to Today
        </button>
      </div>
    );
  }
}
