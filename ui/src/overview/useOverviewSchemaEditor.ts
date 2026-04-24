import { useMutation, type QueryClient } from '@tanstack/react-query';
import type { Connection } from '@xyflow/react';
import { useEffect, useState } from 'react';

import { inferSchemaDocument, saveSchemaDocument } from '../api';
import { formatJson } from '../helpers';
import {
  deriveSchemaEdges,
  parseSchemaConnection,
  parseSchemaDocument,
  upsertSchemaRelationship
} from '../schemaEditor';
import type { OverviewEdge, ResourceOverview, SchemaResponse } from '../types';
import { summarizeSchemaDiff } from '../overviewUtils';
import { invalidateOverviewQueries } from './queryClient';
import { getJsonValidationError } from './shared';
import type { ToastMessage } from './shared';

export function useOverviewSchemaEditor({
  client,
  resources,
  overviewEdges,
  schemaData,
  onToast,
  onOpenSchemaInspector
}: {
  client: QueryClient;
  resources: ResourceOverview[];
  overviewEdges: OverviewEdge[];
  schemaData: SchemaResponse | undefined;
  onToast: (message: string, tone: ToastMessage['tone']) => void;
  onOpenSchemaInspector: () => void;
}) {
  const [schemaDraft, setSchemaDraft] = useState('{}');
  const [schemaDraftDirty, setSchemaDraftDirty] = useState(false);
  const [loadedSchemaText, setLoadedSchemaText] = useState('{}');
  const [schemaStatus, setSchemaStatus] = useState<string | null>(null);

  const saveSchemaMutation = useMutation({
    mutationFn: saveSchemaDocument,
    onSuccess: async () => {
      setSchemaDraftDirty(false);
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
      setSchemaDraftDirty(false);
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
    if (!schemaData || schemaDraftDirty) {
      return;
    }

    const nextText = formatJson(schemaData);
    setLoadedSchemaText(nextText);
    setSchemaDraft(nextText);
  }, [schemaData, schemaDraftDirty]);

  const schemaDiffSummary =
    schemaDraft.trim() !== loadedSchemaText.trim() ? summarizeSchemaDiff(loadedSchemaText, schemaDraft) : [];
  const parsedSchemaDraft = parseSchemaDocument(schemaDraft);
  const hasUsableSchemaDraft = schemaDraftDirty || Boolean(schemaData);
  const schemaEdges =
    hasUsableSchemaDraft && parsedSchemaDraft.document
      ? deriveSchemaEdges(parsedSchemaDraft.document, resources)
      : overviewEdges;
  const schemaValidationError = parsedSchemaDraft.error ?? getJsonValidationError(schemaDraft);

  function updateSchemaDraft(nextDraft: string) {
    setSchemaDraft(nextDraft);
    setSchemaDraftDirty(true);
    setSchemaStatus(null);
  }

  function reloadSchemaDraft() {
    if (schemaData) {
      const nextText = formatJson(schemaData);
      setLoadedSchemaText(nextText);
      setSchemaDraft(nextText);
    }

    setSchemaDraftDirty(false);
    setSchemaStatus('Schema reloaded from the server.');
    void client.invalidateQueries({ queryKey: ['schema'] });
  }

  function saveSchema() {
    if (schemaValidationError) {
      setSchemaStatus(schemaValidationError);
      onToast(schemaValidationError, 'error');
      return;
    }

    saveSchemaMutation.mutate(schemaDraft);
  }

  function stageRelationship(connection: Connection) {
    const parsedConnection = parseSchemaConnection(connection);
    if (!parsedConnection) {
      setSchemaStatus('Drag from a source column on one table to a target column on another table.');
      return;
    }

    if (!parsedSchemaDraft.document) {
      setSchemaStatus(parsedSchemaDraft.error ?? 'Fix the schema draft before creating relationships from the map.');
      return;
    }

    const nextSchema = upsertSchemaRelationship(parsedSchemaDraft.document, parsedConnection);
    updateSchemaDraft(formatJson(nextSchema));
    setSchemaStatus(
      `Staged ${parsedConnection.sourceTable}.${parsedConnection.sourceColumn} -> ${parsedConnection.targetTable}.${parsedConnection.targetColumn}. Save schema to persist it.`
    );
    onOpenSchemaInspector();
  }

  return {
    schemaDraft,
    schemaStatus,
    schemaDiffSummary,
    schemaValidationError,
    schemaEdges,
    schemaBusy: saveSchemaMutation.isPending || inferSchemaMutation.isPending,
    updateSchemaDraft,
    reloadSchemaDraft,
    saveSchema,
    inferSchema() {
      inferSchemaMutation.mutate();
    },
    stageRelationship
  };
}
