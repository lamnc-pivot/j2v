import React, { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import ModelCheckItem from "../components/ModelCheckItem";
import { ModelInfo, ModelStatusResponse } from "../types/model";

interface ModelCheckProps {
  onAllModelsReady: () => void;
}

const DEFAULT_MODELS: ModelInfo[] = [
  { id: "ollama", name: "Ollama", installed: false },
  { id: "qwen", name: "Qwen2.5-7B", installed: false },
  { id: "faster-whisper", name: "Faster-Whisper", installed: false },
  { id: "silero-vad", name: "Silero VAD", installed: false },
  { id: "melotts", name: "MeloTTS", installed: false },
];

const ModelCheck: React.FC<ModelCheckProps> = ({ onAllModelsReady }) => {
  const [models, setModels] = useState<ModelInfo[]>(DEFAULT_MODELS);
  const [loading, setLoading] = useState(true);

  const updateSingleModel = useCallback(
    (modelId: string, updater: (model: ModelInfo) => ModelInfo) => {
      setModels((prevModels) =>
        prevModels.map((model) =>
          model.id === modelId ? updater(model) : model
        )
      );
    },
    []
  );

  const applyStatuses = useCallback((statuses: ModelStatusResponse[]) => {
    setModels(
      statuses.map((status) => ({
        ...status,
        installing: false,
        error: undefined,
      }))
    );
  }, []);

  const checkModelsStatus = useCallback(async () => {
    try {
      setLoading(true);
      console.log("Starting model status check...");

      const statuses = await invoke<ModelStatusResponse[]>("check_model_status");

      console.log("Model status check completed:", statuses);
      console.table(statuses);
      applyStatuses(statuses);
    } catch (error) {
      console.error("Failed to check model status:", error);
    } finally {
      setLoading(false);
    }
  }, [applyStatuses]);

  useEffect(() => {
    void checkModelsStatus();
  }, [checkModelsStatus]);

  const handleInstall = useCallback(async (modelId: string) => {
    updateSingleModel(modelId, (model) => ({
      ...model,
      installing: true,
      error: undefined,
    }));

    try {
      console.log(`Installing ${modelId}...`);
      const result = await invoke("install_model", { modelId });
      console.log(`${modelId} installed successfully:`, result);
      await checkModelsStatus();
    } catch (error) {
      const errorMessage = String(error);
      console.error(`Failed to install ${modelId}:`, errorMessage);

      updateSingleModel(modelId, (model) => ({
        ...model,
        installing: false,
        error: errorMessage,
      }));
    }
  }, [checkModelsStatus, updateSingleModel]);

  const handleMarkInstalled = useCallback((modelId: string) => {
    console.log(`Marked ${modelId} as installed`);

    setModels((prevModels) => {
      const nextModels = prevModels.map((model) =>
        model.id === modelId ? { ...model, installed: true } : model
      );

      const isAllInstalled = nextModels.every((model) => model.installed);
      if (isAllInstalled) {
        console.log("All models installed. Proceeding to main app...");
        queueMicrotask(onAllModelsReady);
      }

      return nextModels;
    });
  }, [onAllModelsReady]);

  const allInstalled = useMemo(
    () => models.every((model) => model.installed),
    [models]
  );

  if (loading) {
    return (
      <div className="screen model-check-screen">
        <div className="model-check-container">
          <h1 className="title">Model Setup</h1>
          <p className="subtitle">Checking models...</p>
          <div className="loading-spinner"></div>
        </div>
      </div>
    );
  }

  return (
    <div className="screen model-check-screen">
      <div className="model-check-container">
        <h1 className="title">Model Setup</h1>
        <p className="subtitle">Please install the required models to continue</p>

        <div className="models-list">
          {models.map((model) => (
            <ModelCheckItem
              key={model.id}
              model={model}
              onInstall={() => handleInstall(model.id)}
              onMarkInstalled={() => handleMarkInstalled(model.id)}
            />
          ))}
        </div>

        <button
          className="btn btn-primary continue-btn"
          onClick={onAllModelsReady}
          disabled={!allInstalled}
        >
          Continue to App
        </button>

        {!allInstalled && (
          <p className="warning-text">Please install all required models to continue</p>
        )}
      </div>
    </div>
  );
};

export default ModelCheck;
