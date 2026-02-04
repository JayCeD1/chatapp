import React from "react";
import { Users, Hash, ArrowRight } from "lucide-react";
import { ChatRoom } from "../types";

interface RoomListProps {
  rooms: ChatRoom[];
  onJoin: (room: ChatRoom) => void;
  username: string;
}

export const RoomList: React.FC<RoomListProps> = ({ rooms, onJoin, username }) => {
  return (
    <div className="w-full max-w-4xl p-8 animate-fade-in z-10">
      <div className="text-center mb-10">
        <h2 className="text-4xl font-bold text-white mb-2 tracking-tight">
          Hello, {username}
        </h2>
        <p className="text-white/70 text-lg">Choose a channel to start collaborating</p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        {rooms.map((room) => (
          <button
            key={room.id}
            onClick={() => onJoin(room)}
            className="group relative overflow-hidden text-left p-6 rounded-2xl bg-white/10 backdrop-blur-md border border-white/10 hover:bg-white/20 transition-all duration-300 hover:scale-[1.02] shadow-xl"
          >
            <div className="absolute inset-0 bg-gradient-to-br from-violet-500/0 via-fuchsia-500/0 to-white/5 opacity-0 group-hover:opacity-100 transition-opacity duration-500" />
            
            <div className="flex justify-between items-start mb-4">
              <div className="p-3 bg-white/10 rounded-xl group-hover:bg-white/20 transition-colors">
                <Hash className="text-white w-6 h-6" />
              </div>
              <div className="flex items-center gap-1.5 px-3 py-1 bg-black/20 rounded-full">
                <Users className="w-3.5 h-3.5 text-emerald-400" />
                <span className="text-xs font-medium text-white/90">
                  {room.user_count || 0} online
                </span>
              </div>
            </div>

            <h3 className="text-xl font-bold text-white mb-2">{room.name}</h3>
            <p className="text-white/60 text-sm mb-4 line-clamp-2">
              {room.description || "No description provided."}
            </p>

            <div className="flex items-center gap-2 text-sm font-medium text-violet-200 group-hover:text-white transition-colors">
              Join Channel <ArrowRight className="w-4 h-4 group-hover:translate-x-1 transition-transform" />
            </div>
          </button>
        ))}
      </div>
    </div>
  );
};
