import { describe, expect, it, vi } from 'vitest';

import { invalidateOverviewQueries } from './queryClient';

describe('invalidateOverviewQueries', () => {
  it('invalidates all overview-related query keys', async () => {
    const client = {
      invalidateQueries: vi.fn().mockResolvedValue(undefined)
    };

    await invalidateOverviewQueries(client as never);

    expect(client.invalidateQueries).toHaveBeenCalledTimes(4);
    expect(client.invalidateQueries).toHaveBeenNthCalledWith(1, { queryKey: ['overview'] });
    expect(client.invalidateQueries).toHaveBeenNthCalledWith(2, { queryKey: ['resource'] });
    expect(client.invalidateQueries).toHaveBeenNthCalledWith(3, { queryKey: ['schema'] });
    expect(client.invalidateQueries).toHaveBeenNthCalledWith(4, { queryKey: ['schema-editor'] });
  });
});
