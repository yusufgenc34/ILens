// Owned fixture for fixed-offset rewriting, EH preservation and Field-RVA ranges.
// The test runner inspects the binary; it never invokes these methods.
namespace ILens.ResourceStrings;
public static class Patterns
{
    public static string Lookup(int value) => value.ToString();
    public static string Message() => Lookup(17);
    public static string WithHandlers()
    {
        try { return Lookup(23); }
        catch (System.Exception) { return Lookup(42); }
        finally { System.GC.KeepAlive(null); }
    }
    public static byte[] FixedData() => new byte[] {
        13,37,2,4,6,8,10,12,14,16,18,20,22,24,26,28
    };
    public static string RuntimeInput(int value) => Lookup(value);
}
