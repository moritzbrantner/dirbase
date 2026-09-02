import { render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { OverviewErrorBoundary } from './OverviewErrorBoundary';

describe('OverviewErrorBoundary', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows a usable recovery surface when a child render throws', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});

    function BrokenOverview() {
      throw new Error('render exploded');
    }

    render(
      <OverviewErrorBoundary>
        <BrokenOverview />
      </OverviewErrorBoundary>
    );

    expect(screen.getByRole('alert')).toHaveTextContent('Overview unavailable');
    expect(screen.getByRole('alert')).toHaveTextContent('render exploded');
    expect(screen.getByRole('button', { name: 'Reload overview' })).toBeVisible();
  });
});
