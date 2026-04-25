import type { QueryClient } from '@tanstack/react-query';

export async function invalidateOverviewQueries(client: QueryClient) {
  await Promise.all([
    client.invalidateQueries({ queryKey: ['overview'] }),
    client.invalidateQueries({ queryKey: ['resource'] }),
    client.invalidateQueries({ queryKey: ['schema'] }),
    client.invalidateQueries({ queryKey: ['schema-editor'] })
  ]);
}
