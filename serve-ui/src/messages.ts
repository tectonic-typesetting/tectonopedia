// See also /src/messages.rs, which defines the message types on the Rust side.

import * as S from "@effect/schema/Schema";
import { formatError } from "@effect/schema/TreeFormatter";
import * as Either from "effect/Either";

const AlertMessage = S.struct({
    file: S.union(S.string, S.null),
    message: S.string,
    context: S.array(S.string),
});
export type AlertMessage = S.Schema.To<typeof AlertMessage>;

const BuildCompleteMessage = S.struct({
    build_complete: S.struct({
        file: S.union(S.string, S.null),
        success: S.boolean,
        elapsed: S.number,
    }),
});
export type BuildCompleteMessage = S.Schema.To<typeof BuildCompleteMessage>;

const BuildStartedMessage = S.struct({
    build_started: S.struct({
        file: S.union(S.string, S.null),
    }),
});
export type BuildStartedMessage = S.Schema.To<typeof BuildStartedMessage>;

const CommandLaunchedMessage = S.struct({
    command_launched: S.string,
});
export type CommandLaunchedMessage = S.Schema.To<typeof CommandLaunchedMessage>;

const ErrorMessage = S.struct({
    error: AlertMessage,
});
export type ErrorMessage = S.Schema.To<typeof ErrorMessage>;

const NoteMessage = S.struct({
    note: AlertMessage,
});
export type NoteMessage = S.Schema.To<typeof NoteMessage>;

const PhaseStartedMessage = S.struct({
    phase_started: S.string,
});
export type PhaseStartedMessage = S.Schema.To<typeof PhaseStartedMessage>;

const ServerInfoMessage = S.struct({
    server_info: S.struct({
        app_port: S.number,
        n_workers: S.number,
    })
});
export type ServerInfoMessage = S.Schema.To<typeof ServerInfoMessage>;

const ServerQuittingMessage = S.literal("server_quitting");
export type ServerQuittingMessage = S.Schema.To<typeof ServerQuittingMessage>;

const ToolOutputStreamStdout = S.literal("stdout");
const ToolOutputStreamStderr = S.literal("stderr");

const ToolOutputStream = S.union(
    ToolOutputStreamStdout,
    ToolOutputStreamStderr,
);

const ToolOutputMessage = S.struct({
    tool_output: S.struct({
        stream: ToolOutputStream,
        lines: S.array(S.string),
    }),
});

export type ToolOutputMessage = S.Schema.To<typeof ToolOutputMessage>;

const WarningMessage = S.struct({
    warning: AlertMessage,
});
export type WarningMessage = S.Schema.To<typeof WarningMessage>;

const YarnServeOutputMessage = S.struct({
    yarn_serve_output: S.struct({
        stream: ToolOutputStream,
        lines: S.array(S.string),
    }),
});

export type YarnServeOutputMessage = S.Schema.To<typeof YarnServeOutputMessage>;

// S.attachPropertySignature might be helpful here but it causes TypeScript
// errors for me.
const Message = S.union(
    BuildCompleteMessage,
    BuildStartedMessage,
    CommandLaunchedMessage,
    ErrorMessage,
    NoteMessage,
    PhaseStartedMessage,
    ServerInfoMessage,
    ServerQuittingMessage,
    ToolOutputMessage,
    WarningMessage,
    YarnServeOutputMessage,
);

export type Message = S.Schema.To<typeof Message>;

const schemaParseMessage = S.decodeUnknownEither(Message);

export function parseMessage(input: any): Message {
    const result = schemaParseMessage(input);

    if (Either.isRight(result)) {
        return result.right;
    }

    throw new Error(`failed to parse input "${JSON.stringify(input)}" as message: ${formatError(result.left)}`);
}