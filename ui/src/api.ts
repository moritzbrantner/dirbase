import type { OverviewPageData, ResourceResponse, SchemaEditorPayload, SchemaResponse } from './types';

type ParsedResponseBody = {
  rawText: string;
  parsed: unknown;
};

async function readJsonOrText(
  response: Response,
  { emptyParsed = null }: { emptyParsed?: unknown } = {}
): Promise<ParsedResponseBody> {
  const rawText = await response.text();
  if (!rawText) {
    return { rawText, parsed: emptyParsed };
  }
  try {
    return { rawText, parsed: JSON.parse(rawText) as unknown };
  } catch {
    return { rawText, parsed: rawText };
  }
}

function apiErrorMessage(parsed: unknown, fallback: string): string {
  if (typeof parsed === 'string' && parsed.trim()) {
    return parsed;
  }
  if (parsed && typeof parsed === 'object') {
    const record = parsed as Record<string, unknown>;
    for (const key of ['message', 'error']) {
      const value = record[key];
      if (typeof value === 'string' && value.trim()) {
        return value;
      }
    }
  }
  return fallback;
}

async function throwApiError(response: Response, fallback: string): Promise<never> {
  const { parsed } = await readJsonOrText(response);
  throw new Error(apiErrorMessage(parsed, fallback));
}

export async function fetchOverview(overviewEndpoint: string): Promise<OverviewPageData> {
  const response = await fetch(overviewEndpoint, {
    headers: {
      Accept: 'application/json'
    }
  });
  if (!response.ok) {
    await throwApiError(
      response,
      `Overview request failed: ${response.status} ${response.statusText}`
    );
  }
  return response.json() as Promise<OverviewPageData>;
}

export async function fetchResource(path: string): Promise<ResourceResponse> {
  const response = await fetch(path, {
    headers: {
      Accept: 'application/json'
    }
  });
  const { rawText, parsed } = await readJsonOrText(response, { emptyParsed: '' });
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
    await throwApiError(
      response,
      `Schema request failed: ${response.status} ${response.statusText}`
    );
  }
  return response.json() as Promise<SchemaResponse>;
}

export async function fetchSchemaEditor(): Promise<SchemaEditorPayload> {
  const response = await fetch('/schema/editor', {
    headers: {
      Accept: 'application/json'
    }
  });
  if (!response.ok) {
    await throwApiError(
      response,
      `Schema editor request failed: ${response.status} ${response.statusText}`
    );
  }
  return response.json() as Promise<SchemaEditorPayload>;
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
    await throwApiError(response, `Schema save failed: ${response.status} ${response.statusText}`);
  }
}

export async function inferSchemaDocument(): Promise<{ path?: string }> {
  const response = await fetch('/schema/infer', {
    method: 'POST',
    headers: {
      Accept: 'application/json'
    }
  });
  if (!response.ok) {
    await throwApiError(response, `Schema infer failed: ${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<{ path?: string }>;
}

export async function mutateResource({
  method,
  path,
  body
}: {
  method: 'POST' | 'PATCH' | 'PUT' | 'DELETE';
  path: string;
  body?: string | null;
}) {
  const response = await fetch(path, {
    method,
    headers: {
      Accept: 'application/json',
      ...(body ? { 'Content-Type': 'application/json' } : {})
    },
    body: body ?? undefined
  });

  const { parsed } = await readJsonOrText(response);

  if (!response.ok) {
    throw new Error(
      apiErrorMessage(parsed, `Request failed: ${response.status} ${response.statusText}`)
    );
  }

  return {
    status: response.status,
    statusText: response.statusText,
    parsed
  };
}
