"""
Example: Python MCP server that extends chisel.

This server exposes domain-specific tools (scaffold_module, run_lint) that
build file content / orchestrate work and delegate all file I/O to a running
chisel instance. The LLM only sees one server.

Prerequisites:
    pip install -r requirements.txt
    MCP_APP_SECRET=mysecret chisel --root /path/to/project   # in another terminal

Run:
    CHISEL_SECRET=mysecret CHISEL_ROOT=/path/to/project python server.py
"""

import asyncio
import os
import textwrap
from contextlib import asynccontextmanager
from typing import AsyncIterator

from mcp import ClientSession
from mcp.client.streamable_http import streamablehttp_client
from mcp.server.fastmcp import FastMCP

# ── chisel connection ──────────────────────────────────────────────────────

CHISEL_URL    = os.environ.get("CHISEL_URL",    "http://127.0.0.1:3000/mcp")
CHISEL_SECRET = os.environ["CHISEL_SECRET"]      # required — fail fast if missing
CHISEL_ROOT   = os.environ.get("CHISEL_ROOT",   "/data")

_HEADERS = {"Authorization": f"Bearer {CHISEL_SECRET}"}


async def _chisel_call(tool: str, **kwargs) -> str:
    """Open a session to chisel, call one tool, return the text result."""
    async with streamablehttp_client(CHISEL_URL, headers=_HEADERS) as (read, write, _):
        async with ClientSession(read, write) as session:
            await session.initialize()
            result = await session.call_tool(tool, kwargs)
            return result.content[0].text


async def chisel_write_file(path: str, content: str) -> str:
    return await _chisel_call("write_file", path=path, content=content)


async def chisel_patch_apply(path: str, patch: str) -> str:
    return await _chisel_call("patch_apply", path=path, patch=patch)


async def chisel_shell_exec(command: str, args: list[str]) -> str:
    return await _chisel_call("shell_exec", command=command, args=args)


# ── your MCP server ────────────────────────────────────────────────────────

mcp = FastMCP("my-project-server")


@mcp.tool()
async def scaffold_module(name: str, directory: str) -> str:
    """
    Scaffold a new Python module with a class stub.

    Args:
        name:      snake_case module name, e.g. user_service
        directory: Path relative to the project root, e.g. src/services
    """
    path    = f"{CHISEL_ROOT}/{directory}/{name}.py"
    class_name = "".join(word.title() for word in name.split("_"))
    content = textwrap.dedent(f"""\
        from dataclasses import dataclass


        @dataclass
        class {class_name}:
            \"\"\"TODO: document {class_name}.\"\"\"

            def run(self) -> None:
                raise NotImplementedError
    """)

    result = await chisel_write_file(path, content)
    return f"Created {path}\n{result}"


@mcp.tool()
async def run_lint(path: str) -> str:
    """
    Run rg (ripgrep) over a file to count TODO markers.
    Delegates to chisel's shell_exec — path is validated server-side.

    Args:
        path: Absolute path to the file to inspect
    """
    result = await chisel_shell_exec("rg", ["--count", "TODO", path])
    return result or "No TODO markers found."


@mcp.tool()
async def apply_patch(path: str, patch: str) -> str:
    """
    Apply a unified diff to a file via chisel (atomic write, hunk-safe).

    Args:
        path:  Absolute path to the target file
        patch: Unified diff string (markdown code fence optional)
    """
    return await chisel_patch_apply(path, patch)


# ── entry point ────────────────────────────────────────────────────────────

if __name__ == "__main__":
    mcp.run()
