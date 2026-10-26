// Owned, synthetic forwarder patterns that resemble call-indirection obfuscation.
// Compiled for static inspection only; never executed by the test runner.
namespace ILens.Proxies;
public static class Calls
{
    // Static forwarder: ldarg.0; ldarg.1; call Real; ret
    public static int Forward(int a, int b) => Real(a, b);
    public static int Real(int a, int b) => a + b;
    public static int UseStatic() => Forward(2, 3);

    // Chained forwarders: UseChain -> Hop1 -> Hop2 -> Real resolve to Real.
    public static int Hop1(int a, int b) => Hop2(a, b);
    public static int Hop2(int a, int b) => Real(a, b);
    public static int UseChain() => Hop1(4, 5);

    // Instance target reached through a static forwarder (callvirt is preserved).
    public static int ForwardScale(Widget w, int x) => w.Scale(x);
    public static int UseInstance(Widget w) => ForwardScale(w, 5);

    // Void forwarder.
    public static void ForwardReset(Widget w) => w.Reset();
    public static void UseVoid(Widget w) => ForwardReset(w);

    // Not a forwarder: does extra work, so it must stay a call to NotProxy.
    public static int NotProxy(int a) => Real(a, 1) + 1;
    public static int UseNotProxy() => NotProxy(7);

    // Argument reordering is not a pure forwarder and must be left intact.
    public static int Swapped(int a, int b) => Real(b, a);
    public static int UseSwapped() => Swapped(1, 2);
}
public sealed class Widget
{
    public int Value;
    public int Scale(int x) => x * 2;
    public void Reset() => Value = 0;
}
