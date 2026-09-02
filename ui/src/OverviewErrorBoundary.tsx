import { Component, type ErrorInfo, type ReactNode } from 'react';

type OverviewErrorBoundaryProps = {
  children: ReactNode;
};

type OverviewErrorBoundaryState = {
  error: Error | null;
};

export class OverviewErrorBoundary extends Component<
  OverviewErrorBoundaryProps,
  OverviewErrorBoundaryState
> {
  state: OverviewErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): OverviewErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Dirbase overview render failed', error, info);
  }

  render() {
    if (!this.state.error) {
      return this.props.children;
    }

    return (
      <main className="overview-page" data-testid="overview-fatal-error">
        <section className="shell-card error-state" role="alert" aria-live="assertive">
          <p className="section-title">Overview unavailable</p>
          <h1>Dirbase is still running, but the dashboard could not render.</h1>
          <p className="overview-copy">
            Reload the page to rebuild the client state. The API remains available independently of
            this dashboard.
          </p>
          <pre className="json-viewer compact">{this.state.error.message}</pre>
          <div>
            <button
              type="button"
              className="overview-secondary-button is-primary"
              onClick={() => window.location.reload()}
            >
              Reload overview
            </button>
          </div>
        </section>
      </main>
    );
  }
}
