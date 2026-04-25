import { useMutation, type QueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useState } from 'react';

import { inferSchemaDocument, saveSchemaDocument } from '../api';
import {
  cloneDeclaredSchema,
  formatDeclaredSchema,
  getSchemaWorkspaceSnapshot,
  mergeSchemaEditorPayload,
  normalizeDeclaredSchema,
  validateSchemaDraft
} from '../schemaWorkspace';
import type { DeclaredSchemaResponse, SchemaEditorPayload, SchemaResponse } from '../types';
import { invalidateOverviewQueries } from './queryClient';
import { getJsonValidationError } from './shared';
import type { ToastMessage } from './shared';

export function useOverviewSchemaWorkspace({
  client,
  schemaEditorData,
  onToast
}: {
  client: QueryClient;
  schemaEditorData: SchemaEditorPayload | undefined;
  onToast: (message: string, tone: ToastMessage['tone']) => void;
}) {
  const snapshot = useMemo(() => getSchemaWorkspaceSnapshot(schemaEditorData), [schemaEditorData]);
  const serverText = useMemo(() => formatDeclaredSchema(snapshot.declared), [snapshot.declared]);
  const [declaredDraft, setDeclaredDraft] = useState<DeclaredSchemaResponse>(() =>
    cloneDeclaredSchema(snapshot.declared)
  );
  const [lastValidDeclaredDraft, setLastValidDeclaredDraft] = useState<DeclaredSchemaResponse>(() =>
    cloneDeclaredSchema(snapshot.declared)
  );
  const [declaredDraftText, setDeclaredDraftText] = useState(serverText);
  const [loadedServerText, setLoadedServerText] = useState(serverText);
  const [schemaStatus, setSchemaStatus] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [stale, setStale] = useState(false);
  const [jsonDrawerOpen, setJsonDrawerOpen] = useState(false);

  const parsedJsonError = getJsonValidationError(declaredDraftText);
  const effectiveSchema = useMemo(
    () => mergeSchemaEditorPayload(snapshot.inferred, lastValidDeclaredDraft),
    [lastValidDeclaredDraft, snapshot.inferred]
  );
  const schemaValidationError = parsedJsonError ?? validateSchemaDraft(snapshot.inferred, lastValidDeclaredDraft);

  const saveSchemaMutation = useMutation({
    mutationFn: saveSchemaDocument,
    onSuccess: async () => {
      setDirty(false);
      setStale(false);
      setSchemaStatus('Schema saved.');
      onToast('Saved schema changes.', 'success');
      await invalidateOverviewQueries(client);
    },
    onError: (error) => {
      const message = error instanceof Error ? error.message : 'Schema save failed.';
      setSchemaStatus(message);
      onToast(message, 'error');
    }
  });
  const inferSchemaMutation = useMutation({
    mutationFn: inferSchemaDocument,
    onSuccess: async (result) => {
      setDirty(false);
      setStale(false);
      setSchemaStatus(`Schema inferred${result.path ? ` to ${result.path}` : ''}.`);
      onToast(`Schema inference completed${result.path ? `: ${result.path}` : '.'}`, 'success');
      await invalidateOverviewQueries(client);
    },
    onError: (error) => {
      const message = error instanceof Error ? error.message : 'Schema infer failed.';
      setSchemaStatus(message);
      onToast(message, 'error');
    }
  });

  useEffect(() => {
    if (!dirty) {
      setDeclaredDraft(cloneDeclaredSchema(snapshot.declared));
      setLastValidDeclaredDraft(cloneDeclaredSchema(snapshot.declared));
      setDeclaredDraftText(serverText);
      setLoadedServerText(serverText);
      setStale(false);
      return;
    }

    if (serverText.trim() !== loadedServerText.trim()) {
      setStale(true);
    }
  }, [dirty, loadedServerText, serverText, snapshot.declared]);

  function applyDeclaredUpdate(nextDeclared: DeclaredSchemaResponse, nextStatus: string | null = null) {
    const normalized = normalizeDeclaredSchema(nextDeclared);
    const nextText = formatDeclaredSchema(normalized);
    setDeclaredDraft(normalized);
    setLastValidDeclaredDraft(normalized);
    setDeclaredDraftText(nextText);
    setDirty(true);
    setStale(false);
    setSchemaStatus(nextStatus);
  }

  function updateDeclaredDraftText(nextText: string) {
    setDeclaredDraftText(nextText);
    setDirty(true);
    setSchemaStatus(null);

    try {
      const parsed = JSON.parse(nextText) as unknown;
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
        return;
      }
      const normalized = normalizeDeclaredSchema(parsed as DeclaredSchemaResponse);
      setDeclaredDraft(normalized);
      setLastValidDeclaredDraft(normalized);
    } catch {
      // Keep the last valid schema preview active while the JSON drawer is invalid.
    }
  }

  function reloadSchemaDraft() {
    const nextDeclared = cloneDeclaredSchema(snapshot.declared);
    setDeclaredDraft(nextDeclared);
    setLastValidDeclaredDraft(nextDeclared);
    setDeclaredDraftText(serverText);
    setLoadedServerText(serverText);
    setDirty(false);
    setStale(false);
    setSchemaStatus('Schema reloaded from the server.');
    void invalidateOverviewQueries(client);
  }

  function discardInvalidJsonChanges() {
    const nextText = formatDeclaredSchema(lastValidDeclaredDraft);
    setDeclaredDraftText(nextText);
    setSchemaStatus('Discarded invalid JSON changes.');
  }

  function saveSchema() {
    if (schemaValidationError) {
      setSchemaStatus(schemaValidationError);
      onToast(schemaValidationError, 'error');
      return;
    }

    saveSchemaMutation.mutate(formatDeclaredSchema(lastValidDeclaredDraft));
  }

  return {
    declaredDraft,
    lastValidDeclaredDraft,
    declaredDraftText,
    effectiveSchema,
    inferredSchema: snapshot.inferred as SchemaResponse,
    loadedServerText,
    savePath: snapshot.savePath,
    schemaBusy: saveSchemaMutation.isPending || inferSchemaMutation.isPending,
    schemaStatus,
    jsonDraftError: parsedJsonError,
    schemaValidationError,
    schemaDirty: dirty,
    schemaStale: stale,
    jsonDrawerOpen,
    setJsonDrawerOpen,
    applyDeclaredUpdate,
    updateDeclaredDraftText,
    reloadSchemaDraft,
    discardInvalidJsonChanges,
    saveSchema,
    inferSchema() {
      inferSchemaMutation.mutate();
    }
  };
}
