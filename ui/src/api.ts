import type { OverviewPageData, ResourceResponse, SchemaResponse } from './types';

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

export async function fetchSchema(): Promise<SchemaResponse> {
  const response = await fetch('/schema', {
    headers: {
      Accept: 'application/json'
    }
  });
  if (!response.ok) {
    throw new Error(`Schema request failed: ${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<SchemaResponse>;
}

export async function saveSchemaDocument(schema: string): Promise<void> {
  const response = await fetch('/schema', {
    method: 'PUT',
    headers: {
      'Content-Type': 'application/json',
      Accept: 'application/json'
    },
    body: schema
  });
  if (!response.ok) {
    const message = await response.text();
    throw new Error(message || `Schema save failed: ${response.status} ${response.statusText}`);
  }
}
