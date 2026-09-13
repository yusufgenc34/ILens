namespace ILens.Editing;

public interface ICounter { int Read(); }
public class Counter : ICounter
{
    public int Value;
    public readonly int ReadOnly = 7;
    public static string Name = "counter";
    public Counter(int value) { Value = value; }
    public virtual int Read() => Value;
    public int Increase(int amount) { Value += amount; return Value; }
    public void Rename(string name) { Name = name; }
    public int Property { get; set; }
}
public sealed class DerivedCounter : Counter { public DerivedCounter(int value) : base(value) { } }
public class Other { public int Value; }
public struct Pair { public int Left; public int Right; }
public static class Operations
{
    public static Counter Create(int value) => new Counter(value);
    public static int Call(Counter counter) => counter.Read();
    public static int InterfaceCall(ICounter counter) => counter.Read();
    public static int DerivedCall(DerivedCounter counter) => counter.Read();
    public static int SetProperty(Counter counter, int value) { counter.Property = value; return counter.Property; }
    public static int StringLength(string text) => text.Length;
    public static object ToObject(Counter counter) => counter;
    public static Counter? AsCounter(object value) => value as Counter;
    public static Counter CastCounter(object value) => (Counter)value;
    public static string[] Strings(string value) { var values = new string[2]; values[0] = value; values[1] = "fixture"; return values; }
    public static object[] Objects(object value) { var values = new object[2]; values[0] = value; values[1] = 42; return values; }
    public static int[][] Jagged(int[] values) { var result = new int[1][]; result[0] = values; return result; }
    public static Counter[] Counters(Counter value) { var result = new Counter[1]; result[0] = value; return result; }
    public static Counter FirstCounter(Counter[] values) => values[0];
    public static object[] Covariant(string[] values) => values;
    public static Counter? Select(bool flag, Counter value) => flag ? value : null;
    public static object SelectKinds(bool flag, Counter value, string text) => flag ? value : text;
    public static bool NotNull(object value) => value != null;
    public static long Length(int[] values) => values.LongLength;
    public static byte ByteAt(byte[] values, int index) => values[index];
    public static short ShortAt(short[] values, int index) => values[index];
    public static long LongAt(long[] values, int index) => values[index];
    public static float FloatAt(float[] values, int index) => values[index];
    public static double DoubleAt(double[] values, int index) => values[index];
    public static nint NativeAt(nint[] values, int index) => values[index];
    public static void StoreByte(byte[] values, byte value) { values[0] = value; }
    public static void StoreShort(short[] values, short value) { values[0] = value; }
    public static void StoreLong(long[] values, long value) { values[0] = value; }
    public static void StoreFloat(float[] values, float value) { values[0] = value; }
    public static void StoreDouble(double[] values, double value) { values[0] = value; }
    public static void StoreNative(nint[] values, nint value) { values[0] = value; }
    public static void ThrowObject(object value) => throw (Exception)value;
    public static T Generic<T>(T value) => value;
    public static int Matrix(int[,] values) => values[0, 0];
    public static int ByRef(ref int value) => value;
    public static Pair Struct(Pair value) => value;
    public static int ExceptionRegion(int value) { try { return 10 / value; } catch { return 0; } }
    public static int OtherRead(Other other) => other.Value;
    public static int ReadOnly(Counter counter) => counter.ReadOnly;
    public static void ThrowException(Exception error) => throw error;
    public static Type RuntimeType(object value) => value.GetType();
}
