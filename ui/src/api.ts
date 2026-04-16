import type { OverviewPageData, ResourceResponse } from './types';

export async function fetchOverview(overviewEndpoint: string): Promise<OverviewPageData> {
  const response = await fetch(overviewEndpoint, {
    headers: {
      Accept: 'application/json'
    }
  });
  if (!response.ok) {
    throw new Error(`Overview request failed: ${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<OverviewPageData>;
}

export async function fetchResource(path: string): Promise<ResourceResponse> {
  const response = await fetch(path, {
    headers: {
      Accept: 'application/json'
    }
  });
  const rawText = await response.text();
  let parsed: unknown = rawText;
  try {
    parsed = JSON.parse(rawText) as unknown;
  } catch {
    // Keep the raw text when the response is not JSON.
  }
  return {
    status: response.status,
    statusText: response.statusText,
    url: response.url,
    rawText,
    parsed
  };
}
