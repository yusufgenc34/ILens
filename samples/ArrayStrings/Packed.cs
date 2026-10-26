// Owned, synthetic array-backed decoders. No commercial protector output or malware is
// included. Compiled for static inspection; these methods must not be invoked by the
// test runner. The byte initializers put this assembly in the Field-RVA layout that the
// general IL writer refuses, so the same-size literal writer has to carry the cleanup.
namespace ILens.ArrayStrings;
public static class Packed
{
    public static string Greeting() => Decode(0);
    public static string Shifted() => Decode(1);
    public static string Malformed() => Decode(255);
    // Static storage copied in by InitializeArray, XOR-decoded, then UTF-8 decoded.
    public static string Decode(int key)
    {
        byte[] data = { 0x4b, 0x66, 0x6f, 0x6f, 0x6c, 0x23, 0x4a, 0x4f, 0x66, 0x6d, 0x70 };
        for (int i = 0; i < data.Length; i++) data[i] = (byte)(data[i] ^ key ^ 3);
        return string.Intern(System.Text.Encoding.UTF8.GetString(data));
    }
    public static string Wide() => Widen(0);
    public static string Widen(int key)
    {
        char[] data = { 'I', 'L', 'e', 'n', 's', ' ', 'w', 'i', 'd', 'e' };
        for (int i = 0; i < data.Length; i++) data[i] = (char)(data[i] ^ key);
        return string.Intern(new string(data));
    }
    public static string Base64() => FromBase64("SGVsbG8gSUxlbnM=");
    public static string FromBase64(string text) =>
        string.Intern(System.Text.Encoding.UTF8.GetString(Convert.FromBase64String(text)));
    // Not a candidate: the argument is not a literal, so no call site is replaced.
    public static string Dynamic(int key) => Decode(key);
    // Not a candidate: a non-interned result would change string reference identity.
    public static string Fresh() => System.Text.Encoding.UTF8.GetString(new byte[] { 0x61 });
}
