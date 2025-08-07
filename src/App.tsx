import { useState } from "react";
import reactLogo from "./assets/react.svg";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import Database from "@tauri-apps/plugin-sql";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  async function loadDb() {
      // This will create the database and run migrations
      const db = await Database.load('sqlite:nutler.db');

      const result = await db.execute("INSERT into users (name, email) VALUES ($1, $2)",["Jesse", "jesse@gmail.com"]);
      console.log(result.rowsAffected);
      const result2 = await db.execute("SELECT * from users");
      console.log(result2);
  }
    async function greet() {

    // Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
    setGreetMsg( await invoke("greet", { name }));
  }

    // Create a user
    async function createUser(name: string, email: string) {
      try {
          const result = await invoke("create_user", { name, email });
          console.log("user created", result);
          return result;
      } catch (error) {
          console.error('Error creating user:', error);
          throw error;
      }

    }

    //Get all users
    async function getUsers(){
      try {
          const result = await invoke("get_users");
          console.log("users", result);
          return result;
      }catch (error) {
          console.error('Error fetching users:', error);
          throw error;
      }

    }

    // Get user by ID
    async function getUserById(id) {
        try {
            const user = await invoke('get_user_by_id', { id });
            console.log('User:', user);
            return user;
        } catch (error) {
            console.error('Error fetching user:', error);
            throw error;
        }
    }


    return (
    <main className="container">
      <h1 className={"text-blue-500 font-bold uppercase"}>Welcome to Tauri + React</h1>

      <div className="row">
        <a href="https://vite.dev" target="_blank">
          <img src="/vite.svg" className="logo vite" alt="Vite logo" />
        </a>
        <a href="https://tauri.app" target="_blank">
          <img src="/tauri.svg" className="logo tauri" alt="Tauri logo" />
        </a>
        <a href="https://react.dev" target="_blank">
          <img src={reactLogo} className="logo react" alt="React logo" />
        </a>
      </div>
      <p className={"uppercase focus:m-auto"}>Click on the Tauri, Vite, and React logos to learn more.</p>

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          getUsers().then((v) => console.log(v));
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
