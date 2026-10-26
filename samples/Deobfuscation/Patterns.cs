// Owned, synthetic patterns. No commercial protector output or malware is included.
// Compiled for static inspection; these methods must not be invoked by the test runner.
namespace ILens.Deobfuscation;
public static class Patterns
{
    public static string Message() => Decode("_r{{x7^[ryd", 23);
    public static string Unicode() => Decode("\ud83d\ude00", 0);
    public static string Dynamic(string text, int key) => Decode(text, key);
    public static string Decode(string text, int key)
    {
        char[] chars = text.ToCharArray();
        for (int i = 0; i < chars.Length; i++) chars[i] = (char)(chars[i] ^ key);
        return string.Intern(new string(chars));
    }
    public static string NonInterned() => Fresh("abc");
    public static string Fresh(string text) => new string(text.ToCharArray());
    public static string ExternalCall() => Read("fixture-do-not-open.txt");
    public static string Read(string name) => System.IO.File.ReadAllText(name);
    public static string Recursive() => Again(1);
    public static string Again(int n) => Again(n);
    public static string Infinite() => Loop(1);
    public static string Loop(int n) { while (n != 0) n ^= 2; return "done"; }
    public static string Allocate() => Make(1000000);
    public static string Make(int count) => string.Intern(new string(new char[count]));
    public static string Initialized() => WithInitializer.Read(1);
    public static string WithHandlers(int n) { try { return Decode("a", n); } catch { return "failed"; } }
    public static int Arithmetic(int value) => value + 1;
    public static int Branch(int value) => value == 7 ? 42 : -1;
    public static string LoneSurrogate() => Decode("\ud800", 0);
}
public static class WithInitializer
{
    static WithInitializer() { Console.WriteLine("This initializer must never run during analysis."); }
    public static string Read(int n) => "initializer-bound";
}
