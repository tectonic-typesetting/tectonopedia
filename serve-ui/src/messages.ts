// See also /src/messages.rs, which defines the message types on the Rust side.

import * as S from "@effect/schema/Schema";
import { formatError } from "@effect/schema/TreeFormatter";
import * as Either from "effect/Either";

const AlertMessage = S.struct({
    message: S.string,
    context: S.array(S.string),
});

const BuildCompleteMessage = S.struct({
    build_complete: S.struct({
        success: S.boolean,
        elapsed: S.number,
    }),
});

const BuildStartedMessage = S.literal("build_started");

const CommandLaunchedMessage = S.struct({
    command_launched: S.string,
});

const ErrorMessage = S.struct({
    error: AlertMessage,
});

const NoteMessage = S.struct({
    note: AlertMessage,
});

const PhaseStartedMessage = S.struct({
    phase_started: S.string,
});

const ServerQuittingMessage = S.literal("server_quitting");
export type ServerQuittingMessage = S.Schema.To<typeof ServerQuittingMessage>;

const ToolOutputStreamStdout = S.literal("stdout");
const ToolOutputStreamStderr = S.literal("stderr");

const ToolOutputStream = S.union(
    ToolOutputStreamStdout,
    ToolOutputStreamStderr,
);

const WarningMessage = S.struct({
    warning: AlertMessage,
});

const YarnOutputMessage = S.struct({
    yarn_output: S.struct({
        stream: ToolOutputStream,
        lines: S.array(S.string),
    }),
});

export type YarnOutputMessage = S.Schema.To<typeof YarnOutputMessage>;

// S.attachPropertySignature might be helpful here but it causes TypeScript
// errors for me.
const Message = S.union(
    BuildCompleteMessage,
    BuildStartedMessage,
    CommandLaunchedMessage,
    ErrorMessage,
    NoteMessage,
    PhaseStartedMessage,
    ServerQuittingMessage,
    WarningMessage,
    YarnOutputMessage,
);

export type Message = S.Schema.To<typeof Message>;

const schemaParseMessage = S.decodeUnknownEither(Message);

export function parseMessage(input: any): Message {
    const result = schemaParseMessage(input);

    if (Either.isRight(result)) {
        return result.right;
    }

    throw new Error(`failed to parse input "${input}" as message: ${formatError(result.left)}`);
}