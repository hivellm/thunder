using System.Text;
using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Accessor and equality semantics of the <see cref="Value"/> model —
/// mirrors the accessor set of rust/thunder-wire/src/value.rs.
/// </summary>
public class ValueTests
{
    [Fact]
    public void AsStr_extracts_only_strings()
    {
        Assert.Equal("x", Value.Str("x").AsStr());
        Assert.Null(Value.Int(1).AsStr());
        Assert.Null(Value.Null.AsStr());
    }

    [Fact]
    public void AsBytes_accepts_str_as_utf8()
    {
        Assert.Equal(new byte[] { 1, 2 }, Value.Bytes(new byte[] { 1, 2 }).AsBytes());
        Assert.Equal(Encoding.UTF8.GetBytes("héllo"), Value.Str("héllo").AsBytes());
        Assert.Null(Value.Int(1).AsBytes());
    }

    [Fact]
    public void AsInt_extracts_only_ints()
    {
        Assert.Equal(42L, Value.Int(42).AsInt());
        Assert.Null(Value.Float(42.0).AsInt());
    }

    [Fact]
    public void AsFloat_widens_ints()
    {
        Assert.Equal(1.5, Value.Float(1.5).AsFloat());
        Assert.Equal(42.0, Value.Int(42).AsFloat());
        Assert.Null(Value.Str("42").AsFloat());
    }

    [Fact]
    public void AsBool_extracts_only_bools()
    {
        Assert.Equal(true, Value.Bool(true).AsBool());
        Assert.Null(Value.Int(1).AsBool());
    }

    [Fact]
    public void AsArray_and_AsMap_extract_collections()
    {
        var array = Value.Array(Value.Int(1), Value.Int(2));
        Assert.Equal(2, array.AsArray()!.Count);
        Assert.Null(array.AsMap());

        var map = Value.Map((Value.Str("k"), Value.Int(1)));
        Assert.Single(map.AsMap()!);
        Assert.Null(map.AsArray());
    }

    [Fact]
    public void MapGet_finds_string_keys_in_insertion_order()
    {
        var map = Value.Map(
            (Value.Int(2), Value.Str("non-string key skipped")),
            (Value.Str("k"), Value.Str("first")),
            (Value.Str("k"), Value.Str("shadowed")));
        Assert.Equal(Value.Str("first"), map.MapGet("k"));
        Assert.Null(map.MapGet("missing"));
        Assert.Null(Value.Int(1).MapGet("k"));
    }

    [Fact]
    public void IsNull_is_true_only_for_null()
    {
        Assert.True(Value.Null.IsNull);
        Assert.False(Value.Int(0).IsNull);
    }

    [Fact]
    public void Float_equality_is_by_bit_pattern()
    {
        Assert.Equal(Value.Float(double.NaN), Value.Float(double.NaN));
        Assert.NotEqual(Value.Float(0.0), Value.Float(-0.0));
    }

    [Fact]
    public void Structural_equality_recurses_into_arrays_and_maps()
    {
        Value Build() => Value.Map(
            (Value.Str("list"), Value.Array(Value.Bool(false), Value.Bytes(new byte[] { 9 }))),
            (Value.Int(7), Value.Null));
        Assert.Equal(Build(), Build());
        Assert.NotEqual(Build(), Value.Map((Value.Str("list"), Value.Array())));
    }

    [Fact]
    public void Implicit_conversions_build_the_expected_variants()
    {
        Value b = true;
        Value i = 42;
        Value f = 1.5;
        Value s = "x";
        Value bytes = new byte[] { 1 };
        Assert.Equal(ValueKind.Bool, b.Kind);
        Assert.Equal(ValueKind.Int, i.Kind);
        Assert.Equal(ValueKind.Float, f.Kind);
        Assert.Equal(ValueKind.Str, s.Kind);
        Assert.Equal(ValueKind.Bytes, bytes.Kind);
    }
}
