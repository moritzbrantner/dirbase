import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  fetchSchemaEditor,
  fetchOverview,
  fetchResource,
  fetchSchema,
  inferSchemaDocument,
  mutateResource,
  saveSchemaDocument
} from './api';

function mockFetch(response: {
  ok: boolean;
  status: number;
  statusText: string;
  json?: unknown;
  text?: string;
  url?: string;
}) {
  const fetchMock = vi.fn(async () => ({
    ok: response.ok,
    status: response.status,
    statusText: response.statusText,
    url: response.url ?? 'http://localhost/test',
    json: async () => response.json,
    text: async () => response.text ?? JSON.stringify(response.json ?? null)
  }));
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('api client', () => {
  it('fetches overview data with an application/json accept header', async () => {
    const payload = {
      schema_enabled: true,
      server_capabilities: {
        readonly: false,
        resource_write: true,
        schema_write: true,
        schema_infer: true
      },
      data_source_kind: 'folder',
      source_label: 'fixtures',
      source_rule: 'Folder mode',
      resource_name_rule: 'json files',
      stats: { resource_count: 0, relation_count: 0, total_rows: 0 },
      resources: [],
      edges: []
    };
    const fetchMock = mockFetch({ ok: true, status: 200, statusText: 'OK', json: payload });

    await expect(fetchOverview('/overview.json')).resolves.toEqual(payload);
    expect(fetchMock).toHaveBeenCalledWith('/overview.json', {
      headers: { Accept: 'application/json' }
    });
  });

  it('throws for failed overview and schema requests', async () => {
    mockFetch({ ok: false, status: 500, statusText: 'Server Error' });
    await expect(fetchOverview('/overview.json')).rejects.toThrow(
      'Overview request failed: 500 Server Error'
    );

    mockFetch({ ok: false, status: 503, statusText: 'Unavailable' });
    await expect(fetchSchema()).rejects.toThrow('Schema request failed: 503 Unavailable');

    mockFetch({ ok: false, status: 502, statusText: 'Bad Gateway' });
    await expect(fetchSchemaEditor()).rejects.toThrow(
      'Schema editor request failed: 502 Bad Gateway'
    );
  });

  it('fetches schema editor payloads', async () => {
    const payload = {
      inferred: { tables: {} },
      declared: null,
      effective: { tables: {} },
      save_path: '/tmp/schema.json'
    };
    const fetchMock = mockFetch({ ok: true, status: 200, statusText: 'OK', json: payload });

    await expect(fetchSchemaEditor()).resolves.toEqual(payload);
    expect(fetchMock).toHaveBeenCalledWith('/schema/editor', {
      headers: { Accept: 'application/json' }
    });
  });

  it('fetches resources and preserves raw text for non-json responses', async () => {
    mockFetch({
      ok: false,
      status: 404,
      statusText: 'Not Found',
      text: 'missing',
      url: 'http://localhost/missing'
    });

    await expect(fetchResource('/missing')).resolves.toEqual({
      status: 404,
      statusText: 'Not Found',
      url: 'http://localhost/missing',
      rawText: 'missing',
      parsed: 'missing'
    });
  });

  it('saves and infers schema documents with server error text surfaced', async () => {
    const saveFetch = mockFetch({ ok: true, status: 200, statusText: 'OK', json: { saved: true } });
    await expect(saveSchemaDocument('{"tables":{}}')).resolves.toBeUndefined();
    expect(saveFetch).toHaveBeenCalledWith('/schema', {
      method: 'PUT',
      headers: {
        'Content-Type': 'application/json',
        Accept: 'application/json'
      },
      body: '{"tables":{}}'
    });

    mockFetch({ ok: false, status: 400, statusText: 'Bad Request', text: 'invalid schema' });
    await expect(saveSchemaDocument('{')).rejects.toThrow('invalid schema');

    mockFetch({
      ok: false,
      status: 422,
      statusText: 'Unprocessable Content',
      json: { message: 'schema message failed' }
    });
    await expect(saveSchemaDocument('{"tables":{}}')).rejects.toThrow('schema message failed');

    mockFetch({ ok: true, status: 200, statusText: 'OK', json: { path: '/tmp/schema.json' } });
    await expect(inferSchemaDocument()).resolves.toEqual({ path: '/tmp/schema.json' });

    mockFetch({ ok: false, status: 500, statusText: 'Server Error', text: 'infer failed' });
    await expect(inferSchemaDocument()).rejects.toThrow('infer failed');
  });

  it('mutates resources with optional JSON bodies and parses success or failure responses', async () => {
    const mutationFetch = mockFetch({
      ok: true,
      status: 201,
      statusText: 'Created',
      text: '{"id":1,"name":"Ada"}'
    });

    await expect(
      mutateResource({ method: 'POST', path: '/users', body: '{"name":"Ada"}' })
    ).resolves.toEqual({
      status: 201,
      statusText: 'Created',
      parsed: { id: 1, name: 'Ada' }
    });
    expect(mutationFetch).toHaveBeenCalledWith('/users', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json'
      },
      body: '{"name":"Ada"}'
    });

    const deleteFetch = mockFetch({ ok: true, status: 204, statusText: 'No Content', text: '' });
    await expect(mutateResource({ method: 'DELETE', path: '/users/1' })).resolves.toEqual({
      status: 204,
      statusText: 'No Content',
      parsed: null
    });
    expect(deleteFetch).toHaveBeenCalledWith('/users/1', {
      method: 'DELETE',
      headers: { Accept: 'application/json' },
      body: undefined
    });

    mockFetch({ ok: false, status: 400, statusText: 'Bad Request', text: 'bad payload' });
    await expect(
      mutateResource({ method: 'PATCH', path: '/users/1', body: '{"bad":true}' })
    ).rejects.toThrow('bad payload');

    mockFetch({
      ok: false,
      status: 409,
      statusText: 'Conflict',
      json: { error: 'conflicting row' }
    });
    await expect(
      mutateResource({ method: 'PUT', path: '/users/1', body: '{"name":"Ada"}' })
    ).rejects.toThrow('conflicting row');
  });
});
