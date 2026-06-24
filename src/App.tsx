import React, { useState } from "react";
import ModelCheck from "./screens/ModelCheck";
import MainApp from "./screens/MainApp";

const App: React.FC = () => {
  const [allModelsReady, setAllModelsReady] = useState(false);

  return (
    <div className="app-container">
      {!allModelsReady ? (
        <ModelCheck onAllModelsReady={() => setAllModelsReady(true)} />
      ) : (
        <MainApp />
      )}
    </div>
  );
};

export default App;
