package dev.horizonlab.parity;
import java.lang.reflect.*;
import java.util.*;
import javax.crypto.Cipher;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.Services;
import net.minecraft.client.Minecraft;
import net.minecraft.client.User;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.ClientHandshakePacketListenerImpl;
import net.minecraft.network.Connection;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.login.ServerboundKeyPacket;
import net.minecraft.util.Crypt;
import com.mojang.authlib.minecraft.SessionService;
import com.mojang.authlib.exceptions.InvalidCredentialsException;
import io.netty.channel.ChannelFutureListener;
import sun.misc.Unsafe;

/** The real login listener permits failed session authentication only for LAN entries. */
public class VanillaLanAuthOracle {
 static class RecordingConnection extends Connection {
  boolean sent, refused;
  RecordingConnection() {super(PacketFlow.CLIENTBOUND);}
  @Override public void send(Packet<?> packet, ChannelFutureListener listener) {sent = true;}
  @Override public void disconnect(Component reason) {refused = true;}
 }
 static void field(Object object, String name, Object value) throws Exception {
  var field = object.getClass().getDeclaredField(name);field.setAccessible(true);field.set(object,value);
 }
 @SuppressWarnings({"unchecked", "rawtypes"})
 public static void main(String[] args) throws Exception {
  SharedConstants.tryDetectVersion();Bootstrap.bootStrap();
  var unsafeField = Unsafe.class.getDeclaredField("theUnsafe");unsafeField.setAccessible(true);
  Unsafe unsafe = (Unsafe)unsafeField.get(null);
  var services = (SessionService)Proxy.newProxyInstance(SessionService.class.getClassLoader(),new Class[]{SessionService.class},(p,m,a)->{
   if(m.getName().equals("joinServer"))throw new InvalidCredentialsException("rejected fixture token");
   return null;
  });
  // Avoid constructing a game window; the listener only needs the user and services.
  Minecraft minecraft = (Minecraft)unsafe.allocateInstance(Minecraft.class);
  field(minecraft,"user",new User("Tester",new UUID(0,1),"fixture",Optional.empty(),Optional.empty()));
  field(minecraft,"services",new Services(services,null,null,null,null));
  var method = ClientHandshakePacketListenerImpl.class.getDeclaredMethod("lambda$handleHello$0",String.class,ServerboundKeyPacket.class,Cipher.class,Cipher.class);method.setAccessible(true);
  var stateClass = Class.forName("net.minecraft.client.multiplayer.ClientHandshakePacketListenerImpl$State");
  var switchState = ClientHandshakePacketListenerImpl.class.getDeclaredMethod("switchState",stateClass);switchState.setAccessible(true);
  for(var type : new ServerData.Type[]{ServerData.Type.LAN,ServerData.Type.OTHER}) {
   var connection = new RecordingConnection();
   var listener = new ClientHandshakePacketListenerImpl(connection,minecraft,new ServerData("fixture","localhost",type),null,false,null,c->{},null,null);
   switchState.invoke(listener,Enum.valueOf((Class)stateClass,"AUTHORIZING"));
   var secret=Crypt.generateSecretKey();var key=Crypt.generateKeyPair();
   var packet=new ServerboundKeyPacket(secret,key.getPublic(),new byte[]{1,2,3,4});
   method.invoke(listener,"fixture",packet,Crypt.getCipher(Cipher.DECRYPT_MODE,secret),Crypt.getCipher(Cipher.ENCRYPT_MODE,secret));
   System.out.println("lan_auth="+type+":"+connection.sent+":"+connection.refused);
  }
 }
}
