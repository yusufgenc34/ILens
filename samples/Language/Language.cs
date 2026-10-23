// C# language constructs for decompiler regression tests. Compiled for static
// inspection only; the test suite never runs this assembly.
using System.Runtime.CompilerServices;
using System.Text;
namespace ILens.Language;

public record Point(int X, int Y);

public struct Money
{
    public decimal Amount;
    public Money(decimal amount) { Amount = amount; }
    public static Money operator +(Money left, Money right) => new(left.Amount + right.Amount);
    public static bool operator ==(Money left, Money right) => left.Amount == right.Amount;
    public static bool operator !=(Money left, Money right) => !(left == right);
    public override bool Equals(object? obj) => obj is Money other && other.Amount == Amount;
    public override int GetHashCode() => Amount.GetHashCode();
}

public class Bag
{
    private readonly List<string> items = new();
    private EventHandler? changed;
    public string this[int index] { get => items[index]; set => items[index] = value; }
    public int Count => items.Count;
    public event EventHandler Changed { add => changed += value; remove => changed -= value; }
    public void Add(params string[] values) { items.AddRange(values); changed?.Invoke(this, EventArgs.Empty); }
}

public static class Extensions
{
    public static string Shout(this string text) => text.ToUpperInvariant() + "!";
}

public static class Constructs
{
    private static readonly object Gate = new();
    private static int counter;

    public static string Ternary(int value) => value > 0 ? "positive" : "other";
    public static bool Both(bool a, bool b, bool c) => a && b || c;
    public static string Coalesce(string? text) => text ?? "none";
    public static int? Length(string? text) => text?.Length;
    public static string Pattern(object value) => value is string s ? s : value is int n && n > 3 ? "big" : "other";
    public static int StringSwitch(string key)
    {
        switch (key)
        {
            case "alpha": return 1;
            case "beta": return 2;
            case "gamma": return 3;
            case "delta": return 4;
            case "epsilon": return 5;
            case "zeta": return 6;
            case "eta": return 7;
            default: return 0;
        }
    }
    public static string SwitchExpression(int value) => value switch { 0 => "zero", 1 => "one", _ => "many" };
    public static string Interpolate(string name, int count) => $"{name} has {count} items";
    public static Point Initialize() => new Point(1, 2) with { Y = 5 };
    public static List<int> Collection() => new List<int> { 1, 2, 3 };
    public static StringBuilder Builder() => new StringBuilder { Capacity = 64 };
    public static void Locked() { lock (Gate) { counter++; } }
    public static unsafe int Pinned(int[] values) { fixed (int* p = values) { return *p; } }
    public static int Checked(int a, int b) => checked(a + b);
    public static (int Sum, int Product) Tuple(int a, int b) => (a + b, a * b);
    public static int Deconstruct(int a, int b) { var (sum, product) = Tuple(a, b); return sum + product; }
    public static bool TryParse(string text, out int value) => int.TryParse(text, out value);
    public static void Swap(ref int a, ref int b) { int t = a; a = b; b = t; }
    public static int Defaulted(int value = 7, string text = "x") => value + text.Length;
    public static string Extension(string text) => text.Shout();
    public static Money Add(Money a, Money b) => a + b;
    public static string Indexer(Bag bag) { bag[0] = "first"; return bag[0]; }
    public static int LocalFunction(int value) { return Twice(value) + 1; static int Twice(int x) => x * 2; }
    public static Func<int, int> StaticLambda() => x => x * 3;
    public static int Linq(int[] values) => values.Where(v => v > 2).Select(v => v * 2).Sum();
    public static IEnumerable<int> Guarded(int n)
    {
        try { for (int i = 0; i < n; i++) yield return i; }
        finally { counter--; }
    }
    public static async IAsyncEnumerable<int> Stream(int n)
    {
        for (int i = 0; i < n; i++) { await Task.Yield(); yield return i; }
    }
    public static async Task<int> AwaitTwice(Task<int> first, Task<int> second) => await first + await second;
    public static string? Nullable(string? text) => text;
    public static int Declared(bool flag)
    {
        int result;
        if (flag) result = 1;
        else result = 2;
        return result;
    }
}
