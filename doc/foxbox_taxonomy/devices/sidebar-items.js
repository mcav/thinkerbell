initSidebarItems({"enum":[["ChannelKind","The kind of the service, i.e. a strongly-typed description of _what_ the service can do. Used both for locating services (e.g. \"I need a clock\" or \"I need something that can provide pictures\") and for determining the data structure that these services can provide or consume."]],"struct":[["Channel","An channel represents a single place where data can enter or leave a device. Note that channels support either a single kind of getter or a single kind of setter. Devices that support both getters or setters, or several kinds of getters, or several kinds of setters, are represented as nodes containing several channels."],["Getter","A getter operation available on a channel."],["Node","Metadata on a node. A node is a device or collection of devices that may offer services. The FoxBox itself a node offering services such as a clock, communication with the user through her smart devices, etc."],["NodeId","A marker for Id. Only useful for writing `Id<NodeId>`."],["Setter","An setter operation available on an channel."]],"trait":[["IOMechanism","The communication mechanism used by the channel."]]});