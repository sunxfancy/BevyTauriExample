import { useEffect, useState } from "react";
import reactLogo from "./assets/react.svg";
import bevyLogo from "./assets/bevy.svg";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";


function FpsDisplay() {

  const [fps, setFps] = useState(0);

  // 使用useEffect设置定时器在组件挂载时启动
  useEffect(() => {
    // 创建定时器每秒更新一次FPS
    const timer = setInterval(async () => {
      const currentFps = await invoke("get_average_frame_rate");
      setFps(currentFps as number);
    }, 1000);

    // 组件卸载时清理定时器
    return () => clearInterval(timer);
  }, []); // 空依赖数组表示只在挂载时运行一次

  return (
    <div>
      FPS: {fps}
    </div>
  );
}


function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function greet() {
    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
    setGreetMsg(await invoke("greet", { name }));
  }


  return (
    <main className="container">
      <div style={{ position: 'absolute', top: '10px', right: '10px' }}>
        <FpsDisplay />
      </div>

      <h1>Welcome to Tauri + React + Bevy</h1>

      <div className="row">
        <a href="https://vitejs.dev" target="_blank">
          <img src="/vite.svg" className="logo vite" alt="Vite logo" />
        </a>
        <a href="https://tauri.app" target="_blank">
          <img src="/tauri.svg" className="logo tauri" alt="Tauri logo" />
        </a>
        <a href="https://reactjs.org" target="_blank">
          <img src={reactLogo} className="logo react" alt="React logo" />
        </a>
        <a href="https://bevyengine.org" target="_blank">
          <img src={bevyLogo} className="logo bevy" alt="Bevy logo" />
        </a>
      </div>
      <p>Click on the Tauri, Vite, React, and Bevy logos to learn more.</p>


      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          greet();
        }}
      >
        <input
          id="greet-input"
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <button type="submit">Greet</button>
      </form>
      <p>{greetMsg}</p>
    </main>
  );
}

export default App;
