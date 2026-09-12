using System.Runtime.InteropServices;
namespace ILens.Samples;

[AttributeUsage(AttributeTargets.Class)]
public sealed class ExampleAttribute(string description) : Attribute { public string Description { get; } = description; }
public interface ICalculator { int Add(int left, int right); }
public enum Mode { Fast, Balanced, Thorough }
public delegate int Transform(int value);

[Example("A real compiled fixture, never executed by the decompiler")]
public class Calculator : ICalculator
{
    public int Offset;
    public static string Greeting = "Hello from local WebAssembly";
    public int Value { get; set; }
    public event EventHandler? Changed;
    public Calculator(int offset) { Offset = offset; }
    public int Add(int left, int right) => left + right;
    public static int DependencyCall(int value) => ILens.Dependency.Arithmetic.Twice(value);
    public static uint UnsignedMax() => uint.MaxValue;
    public static uint UnsignedDivide(uint a, uint b) => a / b;
    public static ulong LargeInteger() => 18446744073709551615UL;
    public static int Multiply(int left, int right) => left * right;
    public int Adjust(int value) => value + Offset;
    public static int Absolute(int value) { if (value < 0) return -value; return value; }
    public static bool IsPositive(int value) => value > 0;
    public static bool BothPositive(int a, int b) => a > 0 && b > 0;
    public static int Sum(int count) { int sum = 0; for (int i = 0; i < count; i++) sum += i; return sum; }
    public static int Countdown(int value) { while (value > 0) value--; return value; }
    public static string Describe(int value) { switch (value) { case 0: return "zero"; case 1: return "one"; case 2: return "two"; case 3: return "three"; default: return "many"; } }
    public static int[] MakeArray(int value) { var items = new int[3]; items[0] = value; items[1] = value + 1; items[2] = 42; return items; }
    public static int First(int[] values) => values[0];
    public static object Box(int value) => value;
    public static int Unbox(object value) => (int)value;
    public static string Cast(object value) => (string)value;
    public static void Fail() => throw new InvalidOperationException("Fixture exception");
    public static int SafeDivide(int a, int b) { try { return a / b; } catch (DivideByZeroException) { return 0; } }
    public void WithFinally() { try { Value++; } finally { Changed?.Invoke(this, EventArgs.Empty); } }
    public static int ReadLength(string path) { using var reader = new StringReader(path); return reader.Read(); }
    public static int Each(int[] values) { int sum=0; foreach (var value in values) sum += value; return sum; }
    public static T Identity<T>(T value) => value;
    public static Func<int, int> MakeAdder(int amount) => value => value + amount;
    public static int? NullableAdd(int? a, int? b) => a + b;
    public static async Task<int> Later(int value) { await Task.Yield(); return value + 1; }
    public static IEnumerable<int> Range(int n) { for (int i=0; i<n; i++) yield return i; }
    [DllImport("unavailable-native-library", EntryPoint="never_call")]
    public static extern int NativeMetadataOnly(int value);
    public class Nested<T> { public T Echo(T value) => value; }
}
