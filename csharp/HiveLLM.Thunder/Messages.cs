namespace HiveLLM.Thunder;

/// <summary>
/// One RPC request (WIRE-001). <see cref="Id"/> is client-chosen and echoed
/// back; many requests multiplex over one connection. Serialized as the
/// array-encoded struct <c>[id, command, args]</c> (WIRE-012); map-shaped
/// requests decode too (WIRE-013).
/// </summary>
public sealed class Request : IEquatable<Request>
{
    /// <summary>Create a request.</summary>
    public Request(uint id, string command, IReadOnlyList<Value>? args = null)
    {
        ArgumentNullException.ThrowIfNull(command);
        Id = id;
        Command = command;
        Args = args ?? System.Array.Empty<Value>();
    }

    /// <summary>Frame id; echoed back by the server (never <see cref="Wire.PushId"/>).</summary>
    public uint Id { get; }

    /// <summary>Command name (e.g. <c>PING</c>).</summary>
    public string Command { get; }

    /// <summary>Positional arguments.</summary>
    public IReadOnlyList<Value> Args { get; }

    /// <summary>Structural equality (args elementwise, floats by bit pattern).</summary>
    public bool Equals(Request? other)
    {
        if (other is null || other.Id != Id ||
            !string.Equals(other.Command, Command, StringComparison.Ordinal) ||
            other.Args.Count != Args.Count)
        {
            return false;
        }

        for (var i = 0; i < Args.Count; i++)
        {
            if (!Args[i].Equals(other.Args[i]))
            {
                return false;
            }
        }

        return true;
    }

    /// <inheritdoc />
    public override bool Equals(object? obj) => Equals(obj as Request);

    /// <inheritdoc />
    public override int GetHashCode() => HashCode.Combine(Id, Command, Args.Count);

    /// <inheritdoc />
    public override string ToString() =>
        $"Request(id: {Id}, command: {Command}, args: [{string.Join(", ", Args)}])";
}

/// <summary>
/// One RPC response (WIRE-001). The result is either a <see cref="Value"/>
/// or a verbatim error string; v1 carries no structured error object —
/// conventions are prefix-based and profile-driven (WIRE-040). Serialized as
/// the array-encoded struct <c>[id, result]</c> with the externally-tagged
/// result <c>{"Ok": value}</c> / <c>{"Err": string}</c> (WIRE-003).
/// </summary>
public sealed class Response : IEquatable<Response>
{
    private Response(uint id, Value? value, string? error)
    {
        Id = id;
        Value = value;
        Error = error;
    }

    /// <summary>Echoed request id, or <see cref="Wire.PushId"/> for push frames.</summary>
    public uint Id { get; }

    /// <summary>True when the result is the Ok arm.</summary>
    public bool IsOk => Error is null;

    /// <summary>The Ok value; null when this is an error response.</summary>
    public Value? Value { get; }

    /// <summary>The verbatim error string; null when this is a success response.</summary>
    public string? Error { get; }

    /// <summary>Success response.</summary>
    public static Response Ok(uint id, Value value)
    {
        ArgumentNullException.ThrowIfNull(value);
        return new Response(id, value, null);
    }

    /// <summary>Error response with the verbatim error string.</summary>
    public static Response Err(uint id, string message)
    {
        ArgumentNullException.ThrowIfNull(message);
        return new Response(id, null, message);
    }

    /// <summary>Structural equality (floats by bit pattern).</summary>
    public bool Equals(Response? other)
    {
        if (other is null || other.Id != Id || other.IsOk != IsOk)
        {
            return false;
        }

        return IsOk
            ? Value!.Equals(other.Value)
            : string.Equals(Error, other.Error, StringComparison.Ordinal);
    }

    /// <inheritdoc />
    public override bool Equals(object? obj) => Equals(obj as Response);

    /// <inheritdoc />
    public override int GetHashCode() => HashCode.Combine(Id, IsOk, Value, Error);

    /// <inheritdoc />
    public override string ToString() =>
        IsOk ? $"Response(id: {Id}, ok: {Value})" : $"Response(id: {Id}, err: \"{Error}\")";
}
