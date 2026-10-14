#!/usr/bin/env python3
"""
Non-blocking Google Drive mount helper for colab-cli.
Unlike `colab drivemount`, this script never hangs on `/dev/tty` when OAuth consent is needed,
and correctly attaches to the existing session's kernel_id/session_id.
"""
import argparse
import json
import sys
from colab_cli.common import state
from colab_cli.auth import get_credentials
from colab_cli.runtime import ColabRuntime
from colab_cli.utils import get_status_code


def check_or_propagate(session_name: str, dryrun: bool = True) -> dict:
    s = state.get_session(session_name)
    url = f"{state.client.colab_domain}/tun/m/credentials-propagation/{s.endpoint}"
    params = {
        "authuser": "0",
        "authtype": "dfs_ephemeral",
        "version": "2",
        "dryrun": "true" if dryrun else "false",
        "propagate": "true",
        "record": "false",
    }
    creds = get_credentials(state.client_oauth_config, provider=state.auth_provider)
    resp = creds.request("GET", url, params=params)
    token = (
        json.loads(resp.text.split("\n", 1)[-1]).get("token")
        if get_status_code(resp) == 200
        else None
    )
    headers = {"x-goog-colab-token": token}
    resp2 = creds.request(
        "POST",
        url,
        params=params,
        headers=headers,
        files={"file_id": (None, "empty.ipynb")},
    )
    data = json.loads(resp2.text.split("\n", 1)[-1])
    data["_xsrf_token"] = token
    data["_url"] = url
    return data


def mount_drive(session_name: str, mount_path: str = "/content/drive") -> int:
    s = state.get_session(session_name)
    runtime = ColabRuntime(
        s.url,
        s.token,
        kernel_id=s.kernel_id,
        session_id=s.session_id,
        session_name=s.name,
        history=state.history,
    )

    auth_result = {"need_user_auth": False, "uri": None}

    def drivefs_hook(deserialize_msg, wsclient):
        content = deserialize_msg.get("content", {})
        if content.get("request", {}).get("authType") == "dfs_ephemeral":
            msg_id = deserialize_msg.get("metadata", {}).get("colab_msg_id")
            dry = check_or_propagate(session_name, dryrun=True)
            if not dry.get("success"):
                auth_result["need_user_auth"] = True
                auth_result["uri"] = dry.get("unauthorized_redirect_uri")
                print(
                    f"\n[mount_drive] REQUIRED: One-time Google Drive Authorization needed for this VM:\n{auth_result['uri']}\n",
                    flush=True,
                )
                return True

            prop = check_or_propagate(session_name, dryrun=False)
            if get_status_code is not None and prop.get("success", True):
                reply = wsclient.session.msg(
                    "input_reply",
                    {"value": {"type": "colab_reply", "colab_msg_id": msg_id}},
                )
                if "header" in deserialize_msg:
                    reply["parent_header"] = deserialize_msg["header"]
                wsclient.stdin_channel.send(reply)
                return True
        return False

    runtime.colab_request_hook = drivefs_hook
    code = f"from google.colab import drive\ndrive.mount('{mount_path}')\n"
    res = runtime.execute(code, allow_stdin=True, stop_on_error=True, timeout=60)
    if auth_result["need_user_auth"]:
        return 2
    if res.status == "error":
        print(f"[mount_drive] Error: {res.ename}: {res.evalue}", file=sys.stderr)
        return 1
    print(f"[mount_drive] Successfully mounted Google Drive at {mount_path}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Non-blocking Google Drive mount for colab-cli")
    parser.add_argument("-s", "--session", default="indextts-l4", help="Colab session name")
    parser.add_argument("--check-only", action="store_true", help="Only check dryrun status and print OAuth URI if needed")
    parser.add_argument("--path", default="/content/drive", help="Mount path on Colab VM")
    args = parser.parse_args()

    if args.check_only:
        data = check_or_propagate(args.session, dryrun=True)
        print(json.dumps({k: v for k, v in data.items() if not k.startswith("_")}, indent=2))
        return 0 if data.get("success") else 2

    return mount_drive(args.session, args.path)


if __name__ == "__main__":
    raise SystemExit(main())
