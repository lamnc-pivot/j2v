import React from "react";
import { ModelInfo } from "../types/model";

interface ModelCheckItemProps {
  model: ModelInfo;
  onInstall: () => void;
  onMarkInstalled: () => void;
}

const ModelCheckItem: React.FC<ModelCheckItemProps> = ({
  model,
  onInstall,
  onMarkInstalled,
}) => {
  const isInstalling = model.installing ?? false;
  const isInstalled = model.installed;
  const statusText = isInstalled
    ? "Installed"
    : isInstalling
      ? "Installing..."
      : "Not installed";

  return (
    <div
      className={`model-item ${isInstalled ? "installed" : ""} ${isInstalling ? "installing" : ""}`}
    >
      <div className="model-info">
        <div className="model-status">
          <span
            className={`status-icon ${isInstalled ? "success" : isInstalling ? "loading" : ""}`}
          >
            {isInstalled ? "✓" : isInstalling ? "⟳" : "○"}
          </span>
          <span className="model-name">{model.name}</span>
        </div>
        <p className="model-status-text">{statusText}</p>
        {model.error && <p className="model-error-text">Error: {model.error}</p>}
      </div>

      <div className="model-actions">
        {!isInstalled ? (
          <>
            <button
              className="btn btn-sm btn-install"
              onClick={onInstall}
              disabled={isInstalling}
            >
              {isInstalling ? "Installing..." : "Install"}
            </button>
            <button
              className="btn btn-sm btn-secondary"
              onClick={onMarkInstalled}
              disabled={isInstalling}
              title="Mark as installed if already present"
            >
              Already Installed
            </button>
          </>
        ) : (
          <span className="installed-badge">Ready</span>
        )}
      </div>
    </div>
  );
};

export default ModelCheckItem;
