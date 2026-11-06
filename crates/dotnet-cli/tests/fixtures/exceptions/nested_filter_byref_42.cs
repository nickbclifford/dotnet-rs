using System;
public class Program {
 static int observed;
 static bool Accept() { return true; }
 static bool Filter() {
  try { throw new Exception(); }
  catch (Exception) when (Accept()) { Console.WriteLine("nested"); }
  return true;
 }
 static void Thrower(ref int local) {
  try { throw new Exception(); }
  finally { local += 12; observed = local; Console.WriteLine(local); }
 }
 static void Worker() { int local = 17; Thrower(ref local); }
 public static int Main() {
  try { Worker(); }
  catch (Exception) when (Filter()) { return observed == 29 ? 42 : 1; }
  return 2;
 }
}
