using System;
public class Program {
 static Exception original = new Exception("identity");
 static void Thrower() { throw original; }
 static bool Filter() {
  try {
   try { Thrower(); }
   catch (Exception) when (Accept()) { Console.WriteLine("rethrow"); throw; }
   finally { Console.WriteLine("finally"); }
  } catch (Exception ex) { Console.WriteLine(Object.ReferenceEquals(ex, original)); }
  return true;
 }
 static bool Accept() { return true; }
 public static int Main() {
  try { Thrower(); }
  catch (Exception) when (Filter()) { Console.WriteLine("outer"); return 42; }
  return 1;
 }
}
