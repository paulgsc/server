### Commands

#### 1. Start the Server

To start the server, simply run the application without any subcommands:

```bash
cargo run --
```

This will initialize the server using the configurations defined in your `.env` file and the `Config` struct. Ensure your environment variables are set up correctly in the `.env` file.

#### 2. Populate Game Clocks

To run the script to populate game clocks, use the following command:

```bash
cargo run -- populate-game-clocks
```

This command will execute the `populate_game_clocks` function asynchronously and populate the game clocks based on your application logic.

### Examples

- **Start the server:**

    ```bash
    cargo run --
    ```

- **Populate game clocks:**

    ```bash
    cargo run -- populate-game-clocks
    ```

## Configuration

Make sure to configure your application by creating a `.env` file in the root directory of the project. Here’s an example of what your `.env` file might contain:

```
DATABASE_URL=your_database_url
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

# TODOs Example

## Setup

1. Declare the database URL

    ```
    export DATABASE_URL="sqlite:todos.db"
    ```

2. Create the database.

    ```
    $ sqlx db create
    ```

3. Run sql migrations

    ```
    $ sqlx migrate run
    ```

## Usage

Add a todo 

```
cargo run -- add "todo description"
```

Complete a todo.

```
cargo run -- done <todo id>
```

List all todos

```
cargo run
```

# SQLite Database Inspection Guide

This guide provides step-by-step instructions on how to inspect your SQLite database using the SQLite3 Command Line Interface (CLI).

## Prerequisites

- SQLite3 CLI installed on your system

## Steps to Inspect Your Database

1. Open your terminal and navigate to the directory containing your SQLite database file.

2. Launch SQLite3 CLI with column headers:
   ```
   sqlite3 -column -header
   ```

3. Open your database file:
   ```
   .open "path/to/your/database.db"
   ```
   Replace `path/to/your/database.db` with the actual path to your SQLite database file.

4. Verify the opened database:
   ```
   .databases
   ```
   This will show the path of the currently opened database.

5. List all tables in the database:
   ```
   .tables
   ```

6. View the schema of a specific table:
   ```
   .schema table_name
   ```
   Replace `table_name` with the name of the table you want to inspect.

7. For a detailed view of columns in a table:
   ```
   PRAGMA table_info(table_name);
   ```
   Replace `table_name` with the name of the table you want to inspect.

8. To exit the SQLite prompt:
   ```
   .quit
   ```

## Example

Here's an example of inspecting the `browser_tabs` table:

```sql
sqlite> .open "/path/to/chrometabs.db"
sqlite> .tables
_sqlx_migrations  browser_tabs
sqlite> .schema browser_tabs
CREATE TABLE browser_tabs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT,
    opener_tab_id INTEGER,
    title TEXT,
    url TEXT,
    pending_url TEXT,
    pinned BOOLEAN,
    highlighted BOOLEAN,
    active BOOLEAN,
    favicon_url TEXT,
    incognito BOOLEAN,
    selected BOOLEAN,
    audible BOOLEAN,
    discarded BOOLEAN,
    auto_discardable BOOLEAN,
    muted_info TEXT,
    width INTEGER,
    height INTEGER,
    last_accessed TIMESTAMP,
    session_id TEXT,
    tab_index INTEGER NOT NULL DEFAULT 0,
    window_id INTEGER NOT NULL DEFAULT 0,
    group_id INTEGER NOT NULL DEFAULT 0
);
```

This README provides a quick reference for inspecting your SQLite database structure using the SQLite3 CLI.
