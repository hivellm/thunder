using System.Globalization;
using System.Text;

namespace HiveLLM.Thunder;

/// <summary>The discriminator of a <see cref="Value"/> (WIRE-002).</summary>
public enum ValueKind
{
    /// <summary>SQL NULL / nil.</summary>
    Null,

    /// <summary>Boolean.</summary>
    Bool,

    /// <summary>Signed 64-bit integer.</summary>
    Int,

    /// <summary>IEEE-754 double; the bit pattern round-trips (WIRE-014).</summary>
    Float,

    /// <summary>Raw bytes, emitted as MessagePack bin (WIRE-010).</summary>
    Bytes,

    /// <summary>UTF-8 string.</summary>
    Str,

    /// <summary>Ordered list of values.</summary>
    Array,

    /// <summary>Insertion-ordered pair list; keys may be any value.</summary>
    Map,
}

/// <summary>
/// The wire value model — the 8 variants byte-compatible with
/// SynapValue / NexusValue / VectorizerValue (WIRE-002). Construct with the
/// static factories; extract with the As* accessors. Equality is structural,
/// with floats compared by IEEE-754 bit pattern (NaN payloads and the -0.0
/// sign bit are wire-significant, WIRE-014).
/// </summary>
public abstract class Value : IEquatable<Value>
{
    private protected Value()
    {
    }

    /// <summary>Which variant this value is.</summary>
    public abstract ValueKind Kind { get; }

    /// <summary>The Null value (unit variant, encoded as the bare string "Null").</summary>
    public static Value Null { get; } = new NullValue();

    /// <summary>Create a Bool value.</summary>
    public static Value Bool(bool value) => value ? BoolValue.True : BoolValue.False;

    /// <summary>Create an Int value (i64).</summary>
    public static Value Int(long value) => new IntValue(value);

    /// <summary>Create a Float value (f64).</summary>
    public static Value Float(double value) => new FloatValue(value);

    /// <summary>Create a Bytes value. The bytes are copied.</summary>
    public static Value Bytes(byte[] value)
    {
        ArgumentNullException.ThrowIfNull(value);
        return new BytesValue((byte[])value.Clone());
    }

    /// <summary>Create a Str value.</summary>
    public static Value Str(string value)
    {
        ArgumentNullException.ThrowIfNull(value);
        return new StrValue(value);
    }

    /// <summary>Create an Array value from the given items.</summary>
    public static Value Array(params Value[] items)
    {
        ArgumentNullException.ThrowIfNull(items);
        return new ArrayValue((Value[])items.Clone());
    }

    /// <summary>Create an Array value from a sequence of items.</summary>
    public static Value Array(IEnumerable<Value> items)
    {
        ArgumentNullException.ThrowIfNull(items);
        return new ArrayValue(items.ToArray());
    }

    /// <summary>Create a Map value from key/value tuples (insertion order preserved).</summary>
    public static Value Map(params (Value Key, Value Val)[] pairs)
    {
        ArgumentNullException.ThrowIfNull(pairs);
        return new MapValue(pairs.Select(p => new KeyValuePair<Value, Value>(p.Key, p.Val)).ToArray());
    }

    /// <summary>Create a Map value from key/value pairs (insertion order preserved).</summary>
    public static Value Map(IEnumerable<KeyValuePair<Value, Value>> pairs)
    {
        ArgumentNullException.ThrowIfNull(pairs);
        return new MapValue(pairs.ToArray());
    }

    /// <summary>True for <see cref="Null"/>.</summary>
    public bool IsNull => Kind == ValueKind.Null;

    /// <summary>Extract the inner string, or null when this is not a Str.</summary>
    public string? AsStr() => this is StrValue s ? s.Value : null;

    /// <summary>
    /// Extract bytes (also accepts Str as its UTF-8 bytes, mirroring the
    /// reference accessor). Null when neither Bytes nor Str.
    /// </summary>
    public byte[]? AsBytes() => this switch
    {
        BytesValue b => b.Value,
        StrValue s => Encoding.UTF8.GetBytes(s.Value),
        _ => null,
    };

    /// <summary>Extract an integer, or null when this is not an Int.</summary>
    public long? AsInt() => this is IntValue i ? i.Value : null;

    /// <summary>Extract a float (accepts Int widened to double), or null.</summary>
    public double? AsFloat() => this switch
    {
        FloatValue f => f.Value,
        IntValue i => i.Value,
        _ => null,
    };

    /// <summary>Extract a bool, or null when this is not a Bool.</summary>
    public bool? AsBool() => this is BoolValue b ? b.Value : null;

    /// <summary>Extract the array items, or null when this is not an Array.</summary>
    public IReadOnlyList<Value>? AsArray() => this is ArrayValue a ? a.Items : null;

    /// <summary>Extract the map pairs, or null when this is not a Map.</summary>
    public IReadOnlyList<KeyValuePair<Value, Value>>? AsMap() => this is MapValue m ? m.Pairs : null;

    /// <summary>Look up a string key in a Map value (first match, insertion order).</summary>
    public Value? MapGet(string key)
    {
        var pairs = AsMap();
        if (pairs is null)
        {
            return null;
        }

        foreach (var pair in pairs)
        {
            if (pair.Key.AsStr() == key)
            {
                return pair.Value;
            }
        }

        return null;
    }

    /// <summary>Structural equality; floats compare by bit pattern.</summary>
    public abstract bool Equals(Value? other);

    /// <inheritdoc />
    public override bool Equals(object? obj) => Equals(obj as Value);

    /// <inheritdoc />
    public abstract override int GetHashCode();

    /// <summary>Structural equality operator.</summary>
    public static bool operator ==(Value? left, Value? right) =>
        left is null ? right is null : left.Equals(right);

    /// <summary>Structural inequality operator.</summary>
    public static bool operator !=(Value? left, Value? right) => !(left == right);

    /// <summary>Convert a bool.</summary>
    public static implicit operator Value(bool value) => Bool(value);

    /// <summary>Convert a long.</summary>
    public static implicit operator Value(long value) => Int(value);

    /// <summary>Convert a double.</summary>
    public static implicit operator Value(double value) => Float(value);

    /// <summary>Convert a string.</summary>
    public static implicit operator Value(string value) => Str(value);

    /// <summary>Convert a byte array.</summary>
    public static implicit operator Value(byte[] value) => Bytes(value);
}

internal sealed class NullValue : Value
{
    public override ValueKind Kind => ValueKind.Null;

    public override bool Equals(Value? other) => other is NullValue;

    public override int GetHashCode() => (int)ValueKind.Null;

    public override string ToString() => "Null";
}

internal sealed class BoolValue : Value
{
    internal static readonly BoolValue True = new(true);
    internal static readonly BoolValue False = new(false);

    internal BoolValue(bool value) => Value = value;

    public bool Value { get; }

    public override ValueKind Kind => ValueKind.Bool;

    public override bool Equals(Value? other) => other is BoolValue b && b.Value == Value;

    public override int GetHashCode() => HashCode.Combine(ValueKind.Bool, Value);

    public override string ToString() => Value ? "Bool(true)" : "Bool(false)";
}

internal sealed class IntValue : Value
{
    internal IntValue(long value) => Value = value;

    public long Value { get; }

    public override ValueKind Kind => ValueKind.Int;

    public override bool Equals(Value? other) => other is IntValue i && i.Value == Value;

    public override int GetHashCode() => HashCode.Combine(ValueKind.Int, Value);

    public override string ToString() => $"Int({Value.ToString(CultureInfo.InvariantCulture)})";
}

internal sealed class FloatValue : Value
{
    internal FloatValue(double value) => Value = value;

    public double Value { get; }

    public override ValueKind Kind => ValueKind.Float;

    public override bool Equals(Value? other) =>
        other is FloatValue f &&
        BitConverter.DoubleToUInt64Bits(f.Value) == BitConverter.DoubleToUInt64Bits(Value);

    public override int GetHashCode() =>
        HashCode.Combine(ValueKind.Float, BitConverter.DoubleToUInt64Bits(Value));

    public override string ToString() =>
        $"Float({Value.ToString("R", CultureInfo.InvariantCulture)})";
}

internal sealed class BytesValue : Value
{
    internal BytesValue(byte[] value) => Value = value;

    public byte[] Value { get; }

    public override ValueKind Kind => ValueKind.Bytes;

    public override bool Equals(Value? other) =>
        other is BytesValue b && b.Value.AsSpan().SequenceEqual(Value);

    public override int GetHashCode()
    {
        var hash = new HashCode();
        hash.Add(ValueKind.Bytes);
        hash.AddBytes(Value);
        return hash.ToHashCode();
    }

    public override string ToString() => $"Bytes({Convert.ToHexString(Value)})";
}

internal sealed class StrValue : Value
{
    internal StrValue(string value) => Value = value;

    public string Value { get; }

    public override ValueKind Kind => ValueKind.Str;

    public override bool Equals(Value? other) =>
        other is StrValue s && string.Equals(s.Value, Value, StringComparison.Ordinal);

    public override int GetHashCode() => HashCode.Combine(ValueKind.Str, Value);

    public override string ToString() => $"Str(\"{Value}\")";
}

internal sealed class ArrayValue : Value
{
    internal ArrayValue(IReadOnlyList<Value> items) => Items = items;

    public IReadOnlyList<Value> Items { get; }

    public override ValueKind Kind => ValueKind.Array;

    public override bool Equals(Value? other)
    {
        if (other is not ArrayValue a || a.Items.Count != Items.Count)
        {
            return false;
        }

        for (var i = 0; i < Items.Count; i++)
        {
            if (!Items[i].Equals(a.Items[i]))
            {
                return false;
            }
        }

        return true;
    }

    public override int GetHashCode()
    {
        var hash = new HashCode();
        hash.Add(ValueKind.Array);
        foreach (var item in Items)
        {
            hash.Add(item);
        }

        return hash.ToHashCode();
    }

    public override string ToString() => $"Array[{string.Join(", ", Items)}]";
}

internal sealed class MapValue : Value
{
    internal MapValue(IReadOnlyList<KeyValuePair<Value, Value>> pairs) => Pairs = pairs;

    public IReadOnlyList<KeyValuePair<Value, Value>> Pairs { get; }

    public override ValueKind Kind => ValueKind.Map;

    public override bool Equals(Value? other)
    {
        if (other is not MapValue m || m.Pairs.Count != Pairs.Count)
        {
            return false;
        }

        for (var i = 0; i < Pairs.Count; i++)
        {
            if (!Pairs[i].Key.Equals(m.Pairs[i].Key) || !Pairs[i].Value.Equals(m.Pairs[i].Value))
            {
                return false;
            }
        }

        return true;
    }

    public override int GetHashCode()
    {
        var hash = new HashCode();
        hash.Add(ValueKind.Map);
        foreach (var pair in Pairs)
        {
            hash.Add(pair.Key);
            hash.Add(pair.Value);
        }

        return hash.ToHashCode();
    }

    public override string ToString() =>
        $"Map[{string.Join(", ", Pairs.Select(p => $"{p.Key}: {p.Value}"))}]";
}
